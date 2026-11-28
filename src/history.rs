use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

use crate::Session;

pub(super) const DEFAULT_HISTSIZE: usize = 500;

fn tail_lines_offset(
    file: &fs::File,
    len: u64,
    mut lines: usize,
    unterminated: bool,
) -> std::io::Result<u64> {
    if lines == 0 {
        return Ok(len);
    }
    let mut pos = len;
    let mut block = [0u8; 16384];
    while pos > 0 {
        let n = pos.min(block.len() as u64) as usize;
        pos -= n as u64;
        file.read_exact_at(&mut block[..n], pos)?;
        if unterminated && pos + n as u64 == len && block[n - 1] != b'\n' {
            lines -= 1;
        }
        for i in memchr::memrchr_iter(b'\n', &block[..n]) {
            if lines == 0 {
                return Ok(pos + i as u64 + 1);
            }
            lines -= 1;
        }
    }
    Ok(0)
}

pub(crate) struct SearchQuery {
    pub text: String,
    pub skip: usize,
    // Earliest prefix produced by this edit, at a UTF-8 boundary.
    pub shortest: usize,
}

enum Hint {
    Local(usize),
    Shared(usize),
    Older,
}

#[derive(Default)]
pub struct History {
    pub(crate) last_search: String,
    displayed: Option<Hint>,
    shared: JournalEntries,
    shared_query: String,
    shared_match: Option<usize>,
    journal: Option<(u64, u64)>,
    snapshot: (u64, i64, i64),
    offset: u64,
    tail_start: u64,
    older_prefix: String,
    older_match: Option<String>,
    search_buffer: Vec<u8>,
    window: window::Window,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    Unread,
    Recent,
    All,
}

fn default_history(session: &Session, path: &std::path::Path) -> bool {
    path.file_name().is_some_and(|s| s == ".bash_history")
        && session
            .personal_home()
            .is_some_and(|home| path.parent() == Some(std::path::Path::new(home.as_ref())))
}

fn custom_history(session: &Session) -> Option<String> {
    let path = session.scalar_setting("HISTFILE")?;
    if path.is_empty()
        || path == "/dev/null"
        || default_history(session, std::path::Path::new(path.as_ref()))
    {
        return None;
    }
    Some(path.into_owned())
}

pub(crate) fn load_script_history(session: &mut Session) {
    if let Some(path) = custom_history(session)
        && fs::metadata(&path).is_ok_and(|m| m.is_file())
    {
        let _ = commands::history_file(session, 'r', &[path]);
    }
}

pub(crate) fn start_interactive(session: &mut Session) {
    if let Some(path) = custom_history(session) {
        if fs::metadata(&path).is_ok_and(|m| m.is_file())
            && let Err(e) = commands::history_file(session, 'r', &[path])
        {
            diagnostic!("history: {e}");
        }
    } else {
        session.initial_history = InitialJournal::snapshot(session).map(std::rc::Rc::new);
    }
}

pub(crate) fn finish_interactive(session: &mut Session) {
    if session
        .history
        .as_ref()
        .is_none_or(|h| h.written == h.lines.len())
    {
        return;
    }
    if let Some(path) = custom_history(session) {
        let limit = session.get("HISTFILESIZE").and_then(|s| s.parse().ok());
        let timestamped = session.get("HISTTIMEFORMAT").is_some();
        let incomplete = session
            .history
            .as_ref()
            .is_some_and(|h| h.incomplete(std::path::Path::new(&path)));
        let limit = limit.filter(|_| !incomplete);
        let op = if session.shopt.histappend || incomplete {
            'a'
        } else {
            'w'
        };
        if let Err(e) =
            commands::history_file(session, op, std::slice::from_ref(&path)).and_then(|()| {
                limit.map_or(Ok(()), |n| {
                    commands::limit_plain_file(std::path::Path::new(&path), n, timestamped, None)
                })
            })
        {
            diagnostic!("history: {e}");
        }
    }
}

fn lock_exclusive(file: &fs::File) -> std::io::Result<()> {
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

// Open nonblocking before checking the descriptor: a renamed journal can be a FIFO.
fn open_journal(path: &std::path::Path) -> std::io::Result<(fs::File, fs::Metadata)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    Ok((file, metadata))
}

fn file(session: &Session) -> Option<PathBuf> {
    let setting = session.scalar_setting("HISTFILE")?;
    if setting.is_empty()
        || setting == "/dev/null"
        || session.scalar_setting("HISTFILESIZE").as_deref() == Some("0")
    {
        return None;
    }
    let history = std::path::Path::new(setting.as_ref());
    if !default_history(session, history) {
        return None;
    }
    let dir = session
        .get("XDG_STATE_HOME")
        .filter(|v| PathBuf::from(v).is_absolute())
        .map(PathBuf::from)
        .or_else(|| {
            session
                .personal_home()
                .map(|h| std::path::Path::new(h.as_ref()).join(".local/state"))
        })?;
    Some(dir.join("tonari/history"))
}

fn encode(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
}

// Commands share one allocation, so loading or discarding a million journal
// records does not require a million separate heap operations.
#[derive(Default)]
struct JournalEntries {
    text: String,
    ends: Vec<usize>,
}

impl JournalEntries {
    fn clear(&mut self) {
        self.text.clear();
        self.ends.clear();
    }

    fn get(&self, index: usize) -> &str {
        let start = index.checked_sub(1).map_or(0, |i| self.ends[i]);
        &self.text[start..self.ends[index]]
    }

    fn push(&mut self, value: &str) {
        let start = self.text.len();
        if !decode_into(value, &mut self.text) {
            self.text.truncate(start);
            return;
        }
        self.ends.push(self.text.len());
    }
}

// Versions share pid, status, directory and command fields. Timestamped
// journals add a Unix second after the version. Search skips it as text.
fn journal_fields(row: &str) -> Option<(&str, Option<&str>)> {
    if let Some(body) = row.strip_prefix("1\t") {
        return Some((body, None));
    }
    let (timestamp, body) = row.strip_prefix("2\t")?.split_once('\t')?;
    Some((body, Some(timestamp)))
}

struct JournalRecord<'a> {
    time: Option<&'a str>,
    pid: &'a str,
    status: &'a str,
    dir: &'a str,
    command: &'a str,
}

fn journal_record(row: &str) -> Option<JournalRecord<'_>> {
    let (body, time) = journal_fields(row)?;
    let mut fields = body.splitn(4, '\t');
    Some(JournalRecord {
        time,
        pid: fields.next()?,
        status: fields.next()?,
        dir: fields.next()?,
        command: fields.next()?,
    })
}

fn command_field(row: &str) -> Option<&str> {
    Some(journal_record(row)?.command)
}

// Old journals and unmarked Bash files may have no creation timestamp.
const UNKNOWN_TIME: i64 = i64::MIN;

fn decode(value: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    decode_into(value, &mut out).then_some(out)
}

fn decode_into(value: &str, out: &mut String) -> bool {
    let mut rest = value;
    while let Some((literal, escaped)) = rest.split_once('\\') {
        out.push_str(literal);
        let byte = match escaped.as_bytes().first() {
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'\\') => '\\',
            _ => {
                return false;
            }
        };
        out.push(byte);
        rest = &escaped[1..];
    }
    out.push_str(rest);
    true
}

pub(crate) mod arguments;
mod filter;
pub(crate) use filter::{Filter, fragments};
pub(crate) mod commands;
pub(crate) mod expand;
mod window;

// Keep the live window contiguous for prefix and reverse searches. Dropping
// an entry frees its string immediately, but moves the live suffix in batches.
#[derive(Clone, Default)]
struct HistoryWindow<T> {
    values: Vec<T>,
    head: usize,
}

impl<T> std::ops::Deref for HistoryWindow<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.values[self.head..]
    }
}

impl<T> std::ops::DerefMut for HistoryWindow<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.values[self.head..]
    }
}

impl<T: Default> HistoryWindow<T> {
    fn push(&mut self, value: T) {
        self.values.push(value);
    }

    fn remove(&mut self, start: usize, end: usize) {
        if start == 0 {
            self.discard(end);
        } else {
            self.values.drain(self.head + start..self.head + end);
        }
    }

    fn discard(&mut self, count: usize) {
        self.head += count;
        if self.head >= self.len() || self.head >= 256 && self.head >= self.len() / 4 {
            self.values.drain(..self.head);
            self.head = 0;
        } else {
            self.values[self.head - count..self.head].fill_with(T::default);
        }
    }

    fn prepend(&mut self, mut prefix: Vec<T>) {
        prefix.extend(self.values.drain(self.head..));
        self.values = prefix;
        self.head = 0;
    }
}

#[derive(Clone, Default)]
pub(crate) struct Entries {
    lines: HistoryWindow<String>,
    erase_counts: Option<foldhash::HashMap<u64, usize>>,
    timestamps: HistoryWindow<libc::time_t>,
    base: usize,
    current: Option<usize>,
    current_lines: usize,
    // A later history -r/-n can interleave imported entries with local edits.
    pending: HistoryWindow<bool>,
    written: usize,
    incomplete_files: Vec<(u64, u64)>,
    imported: Option<InitialJournal>,
}

#[derive(Clone)]
pub(crate) struct InitialJournal {
    path: PathBuf,
    identity: (u64, u64),
    len: u64,
}

impl InitialJournal {
    pub(crate) fn snapshot(session: &Session) -> Option<Self> {
        let path = file(session)?;
        let m = fs::metadata(&path).ok()?;
        if session
            .history
            .as_deref()
            .and_then(|h| h.imported.as_ref())
            .is_some_and(|seen| seen.path == path && seen.identity == (m.dev(), m.ino()))
        {
            return None;
        }
        (m.is_file() && m.len() > 0).then(|| Self {
            path,
            identity: (m.dev(), m.ino()),
            len: m.len(),
        })
    }

    // The editor already reads the shared journal in windows. History builtins
    // need only the startup suffix permitted by HISTSIZE, on first access.
    fn load(&self, session: &mut Session) -> std::io::Result<()> {
        let (mut journal, m) = open_journal(&self.path)?;
        if (m.dev(), m.ino()) != self.identity || m.len() < self.len {
            return Ok(());
        }
        let available = session
            .history_limit("HISTSIZE", DEFAULT_HISTSIZE)
            .saturating_sub(session.history_lines().len());
        let pos = tail_lines_offset(&journal, self.len, available, false)?;
        journal.seek(SeekFrom::Start(pos))?;
        let mut older = Vec::new();
        let mut timestamps = Vec::new();
        let mut reader = BufReader::new(journal.take(self.len - pos));
        let mut row = Vec::new();
        let mut complete = pos;
        while reader.read_until(b'\n', &mut row)? != 0 {
            let Some(record) = row.strip_suffix(b"\n") else {
                break;
            };
            complete += row.len() as u64;
            if let Some(record) = std::str::from_utf8(record).ok().and_then(journal_record)
                && let Some(text) = decode(record.command)
            {
                older.push(text);
                timestamps.push(
                    record
                        .time
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(UNKNOWN_TIME),
                );
            }
            row.clear();
        }
        let h = session.history_mut();
        let n = older.len();
        h.erase_counts = None;
        h.lines.prepend(older);
        h.current = h.current.map(|index| index + n);
        h.timestamps.prepend(timestamps);
        h.pending.prepend(vec![false; n]);
        h.written += n;
        h.imported = Some(Self {
            len: complete,
            ..self.clone()
        });
        Ok(())
    }
}

struct HistoryInsertion {
    erase_duplicates: bool,
}

impl Session {
    pub(crate) fn history_number(&self) -> usize {
        self.history
            .as_ref()
            .map_or(1, |h| h.base + h.current.unwrap_or(h.lines.len()) + 1)
    }
    pub(crate) fn history_command_number(&self) -> usize {
        self.history.as_ref().map_or(0, |h| h.base + h.lines.len())
    }
    pub(crate) fn history_lines(&self) -> &[String] {
        self.history.as_deref().map_or(&[], |h| &h.lines[..])
    }
    fn history_mut(&mut self) -> &mut Entries {
        std::rc::Rc::make_mut(self.history.get_or_insert_with(Default::default))
    }
    pub(crate) fn apply_history_limit(&mut self, name: &str, value: &str) {
        let Ok(limit) = value.parse::<usize>() else {
            return;
        };
        if name == "HISTSIZE" {
            if let Some(history) = self.history.as_mut() {
                std::rc::Rc::make_mut(history).trim_to(limit);
            }
        } else if let Some(path) = self
            .get("HISTFILE")
            .filter(|s| !s.is_empty() && s != "/dev/null")
        {
            let path = std::path::Path::new(&path);
            // The default history is Tonari's shared journal. The user's Bash
            // file remains independent. Explicit plain history uses line limits.
            if default_history(self, path) {
                return;
            }
            let timed = self.get("HISTTIMEFORMAT").is_some();
            let history = self.history.as_mut().map(std::rc::Rc::make_mut);
            if history.as_ref().is_some_and(|h| h.incomplete(path)) {
                return;
            }
            if let Err(e) = commands::limit_plain_file(
                path,
                limit,
                timed,
                history.and_then(|h| h.imported.as_mut()),
            ) && e.kind() != std::io::ErrorKind::NotFound
            {
                diagnostic!("{}: {e}", path.display());
            }
        }
    }

    fn history_limit(&self, key: &str, default: usize) -> usize {
        self.scalar_setting(key)
            .and_then(|s| s.parse::<i64>().ok())
            .map_or(default, |n| usize::try_from(n).unwrap_or(usize::MAX))
    }
    pub(crate) fn begin_history_line(&mut self, text: &str) -> bool {
        if !self.history_enabled {
            return false;
        }
        let limit = self.history_limit("HISTSIZE", DEFAULT_HISTSIZE);
        if limit == 0 {
            return false;
        }
        let text = text.strip_suffix('\n').unwrap_or(text);
        let normalized;
        let text = if self.shopt.cmdhist && !self.shopt.lithist && text.contains('\n') {
            normalized = crate::parser::history_command(text);
            &normalized
        } else {
            text
        };
        let Some(policy) = self.history_insertion(text, self.history_lines().last()) else {
            return false;
        };
        if !text.contains('\n') || self.shopt.cmdhist {
            self.push_history_limited(text, true, limit, &policy);
        } else {
            let mut count = 0;
            for line in text.split('\n').filter(|s| !s.is_empty()) {
                self.push_history_limited(line, true, limit, &policy);
                count += 1;
            }
            self.history_mut().current_lines = count;
        }
        true
    }
    #[cfg(test)]
    fn push_history(&mut self, text: &str, current: bool) {
        let limit = self.history_limit("HISTSIZE", DEFAULT_HISTSIZE);
        self.push_history_limited(
            text,
            current,
            limit,
            &HistoryInsertion {
                erase_duplicates: false,
            },
        );
    }

    fn push_history_limited(
        &mut self,
        text: &str,
        current: bool,
        limit: usize,
        policy: &HistoryInsertion,
    ) {
        if limit == 0 {
            return;
        }
        let h = self.history_mut();
        h.append_line(text.into(), policy.erase_duplicates, true);
        h.timestamps
            .push(unsafe { libc::time(std::ptr::null_mut()) });
        h.trim_to(limit);
        h.current = current.then(|| h.lines.len() - 1);
        h.current_lines = usize::from(current);
    }
    pub(crate) fn finish_history_line(&mut self) {
        if let Some(data) = &mut self.history {
            std::rc::Rc::make_mut(data).current = None;
        }
    }

    pub(crate) fn persist_current_history(
        &mut self,
        cwd: Option<&str>,
        status: u8,
        history: &mut History,
    ) {
        let Some(h) = self.history.as_deref() else {
            return;
        };
        let Some(index) = h.current else { return };
        let start = (index + 1).saturating_sub(h.current_lines.max(1));
        if self.history_enabled {
            for line in &h.lines[start..index] {
                History::append(self, line, cwd, status, Some(&mut history.window));
            }
        }
        let line = &h.lines[index];
        if self.history_enabled && self.accept_history(line, h.lines[..index].last()) {
            History::append(self, line, cwd, status, Some(&mut history.window));
        } else {
            self.history_mut().discard_current();
        }
    }

    fn accept_history(&self, source: &str, previous: Option<&String>) -> bool {
        self.history_limit("HISTSIZE", DEFAULT_HISTSIZE) > 0
            && self.history_insertion(source, previous).is_some()
    }

    fn record_replay(&mut self, text: &str) -> bool {
        let limit = self.history_limit("HISTSIZE", DEFAULT_HISTSIZE);
        if limit != 0
            && let Some(policy) = self.history_insertion(text, self.history_lines().last())
        {
            self.push_history_limited(text, false, limit, &policy);
            return true;
        }
        false
    }

    fn history_insertion(
        &self,
        source: &str,
        previous: Option<&String>,
    ) -> Option<HistoryInsertion> {
        let session = self;
        let text = source.strip_suffix('\n').unwrap_or(source);
        if text.is_empty() || session.interactive && text.trim().is_empty() {
            return None;
        }
        let previous = previous.map(String::as_str);
        let control = session.scalar_setting("HISTCONTROL").unwrap_or_default();
        let mut erase_duplicates = false;
        for rule in control.split(':') {
            if rule == "erasedups" {
                erase_duplicates = true;
            }
            if matches!(rule, "ignorespace" | "ignoreboth") && text.starts_with(' ') {
                return None;
            }
            if matches!(rule, "ignoredups" | "ignoreboth") && previous == Some(text) {
                return None;
            }
        }
        if session.scalar_setting("HISTIGNORE").is_some_and(|s| {
            s.split(':').any(|p| {
                !p.is_empty()
                    && (if p == "&" {
                        previous == Some(text)
                    } else {
                        crate::pattern::matches(text, p)
                    })
            })
        }) {
            return None;
        }
        Some(HistoryInsertion { erase_duplicates })
    }
}

impl Entries {
    fn mark_incomplete(&mut self, identity: (u64, u64)) {
        if !self.incomplete_files.contains(&identity) {
            self.incomplete_files.push(identity);
        }
    }

    fn incomplete(&self, path: &std::path::Path) -> bool {
        !self.incomplete_files.is_empty()
            && fs::metadata(path).is_ok_and(|m| self.incomplete_files.contains(&(m.dev(), m.ino())))
    }

    fn append_line(&mut self, line: String, erase: bool, pending: bool) {
        use std::hash::BuildHasher;
        if erase {
            let counts = self.erase_counts.get_or_insert_with(|| {
                let mut counts = foldhash::HashMap::default();
                for entry in self.lines.iter() {
                    let hash = counts.hasher().hash_one(entry);
                    *counts.entry(hash).or_default() += 1;
                }
                counts
            });
            let hash = counts.hasher().hash_one(&line);
            let mut remaining = counts.get(&hash).copied().unwrap_or(0);
            if remaining == 1 && self.lines.first() == Some(&line) {
                self.remove(0);
                remaining = 0;
            }
            let mut i = self.lines.len();
            while remaining != 0 && i != 0 {
                i -= 1;
                if self.lines[i] == line {
                    self.remove(i);
                    remaining -= 1;
                }
            }
            *self.erase_counts.as_mut().unwrap().entry(hash).or_default() += 1;
        } else {
            self.erase_counts = None;
        }
        if !pending && self.written == self.lines.len() {
            self.written += 1;
        }
        self.pending.push(pending);
        self.lines.push(line);
    }
    fn forget_line(&mut self, i: usize) {
        use std::hash::BuildHasher;
        if let Some(counts) = &mut self.erase_counts {
            let key = counts.hasher().hash_one(&self.lines[i]);
            if let std::collections::hash_map::Entry::Occupied(mut entry) = counts.entry(key) {
                *entry.get_mut() -= 1;
                if *entry.get() == 0 {
                    entry.remove();
                }
            }
        }
    }
    fn trim_to(&mut self, limit: usize) {
        if self.lines.len() > limit {
            let count = self.lines.len() - limit;
            if self.erase_counts.is_some() {
                for i in 0..count {
                    self.forget_line(i);
                }
            }
            self.lines.discard(count);
            self.timestamps.discard(count);
            self.pending.discard(count);
            self.base += count;
            self.written = self.written.saturating_sub(count);
            self.current = self.current.and_then(|index| index.checked_sub(count));
        }
    }

    fn remove(&mut self, index: usize) {
        self.remove_range(index, index + 1);
    }

    fn remove_range(&mut self, start: usize, end: usize) {
        if self.erase_counts.is_some() {
            for i in start..end {
                self.forget_line(i);
            }
        }
        self.lines.remove(start, end);
        self.timestamps.remove(start, end);
        self.pending.remove(start, end);
        self.written -= end.min(self.written).saturating_sub(start);
        self.current = self.current.and_then(|j| {
            if (start..end).contains(&j) {
                None
            } else {
                Some(j - if j >= end { end - start } else { 0 })
            }
        });
    }
    fn discard_current(&mut self) {
        if let Some(i) = self.current {
            self.remove(i);
        }
    }
}

// Only complete records are passed to this matcher. A hit in the PID or
// directory, or two hits in one command, never consume a second history entry.
fn encoded_fragment(needle: &str) -> std::borrow::Cow<'_, str> {
    // A tab or CR can also occur literally in the command field of an
    // existing journal. Either spelling still contains this encoded part.
    let part = needle.split(['\t', '\r']).max_by_key(|s| s.len()).unwrap();
    if part.contains(['\\', '\n']) {
        let mut encoded = String::new();
        encode(part, &mut encoded);
        std::borrow::Cow::Owned(encoded)
    } else {
        std::borrow::Cow::Borrowed(part)
    }
}

fn journal_matcher<'a>(
    needle: &'a str,
    prefix: bool,
    raw: &'a str,
) -> impl Fn(&[u8], &mut usize) -> Option<String> + 'a {
    let at_start = prefix && !needle.contains(['\t', '\r']);
    let non_ascii = raw.bytes().find(|b| !b.is_ascii());
    let finder = memchr::memmem::Finder::new(raw.as_bytes());
    let reverse = memchr::memmem::FinderRev::new(raw.as_bytes());
    move |rows: &[u8], skip: &mut usize| {
        let raw = finder.needle();
        // Only candidate blocks need record parsing and UTF-8 validation.
        if non_ascii.is_none_or(|b| memchr::memchr(b, rows).is_some())
            && finder.find(rows).is_some()
        {
            let mut remaining = rows;
            while !remaining.is_empty() {
                let Some(hit) = reverse.rfind(remaining) else {
                    break;
                };
                let start = memchr::memrchr(b'\n', &remaining[..hit]).map_or(0, |i| i + 1);
                let end =
                    memchr::memchr(b'\n', &remaining[hit..]).map_or(remaining.len(), |i| hit + i);
                let line = &remaining[start..end];
                // Skip the entire record, including further hits in its
                // metadata or the same command, before looking for an older hit.
                remaining = &remaining[..start.saturating_sub(1)];
                if let Ok(line) = std::str::from_utf8(line)
                    && let Some(word) = command_field(line)
                    && if at_start {
                        word.as_bytes().starts_with(raw)
                    } else {
                        finder.find(word.as_bytes()).is_some()
                    }
                    && let Some(command) = decode(word)
                    && if prefix {
                        command.len() > needle.len() && command.starts_with(needle)
                    } else {
                        command.contains(needle)
                    }
                {
                    if *skip == 0 {
                        return Some(command);
                    }
                    *skip -= 1;
                }
            }
        }
        None
    }
}

impl History {
    pub fn persist(session: &Session, source: &str, cwd: Option<&str>, status: u8) {
        Self::append(session, source, cwd, status, None);
    }

    pub fn prepare_search(&mut self, session: &Session, waiting: &dyn Fn() -> bool) {
        self.window.prepare(session, waiting);
    }

    fn append(
        session: &Session,
        source: &str,
        cwd: Option<&str>,
        status: u8,
        window: Option<&mut window::Window>,
    ) {
        let text = source.strip_suffix('\n').unwrap_or(source);
        let Some(path) = file(session) else {
            return;
        };
        let dir = path.parent().unwrap();
        if !dir.is_dir()
            && fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .is_err()
        {
            return;
        }
        let Ok(mut journal) = OpenOptions::new()
            .append(true)
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .create(true)
            .mode(0o600)
            .open(path)
        else {
            return;
        };
        if lock_exclusive(&journal).is_err() {
            return;
        }
        let Ok(meta) = journal.metadata() else { return };
        if !meta.is_file() {
            return;
        }
        // One line per entry, including pastes with embedded tabs and newlines.
        let mut entry = if session.get("HISTTIMEFORMAT").is_some() {
            let timestamp = session
                .history
                .as_ref()
                .and_then(|h| h.timestamps.last().copied())
                .unwrap_or_else(|| unsafe { libc::time(std::ptr::null_mut()) });
            format!("2\t{timestamp}\t{}\t{status}\t", std::process::id())
        } else {
            format!("1\t{}\t{status}\t", std::process::id())
        };
        encode(cwd.unwrap_or(""), &mut entry);
        entry.push('\t');
        encode(text, &mut entry);
        entry.push('\n');
        // A writer that exited mid-record left no complete newline. Discard
        // just that suffix while holding the same lock used by every writer.
        let mut len = meta.len();
        // A matching copy already contains the committed final newline. Its
        // stamp is checked against the metadata obtained under the writer lock.
        let tracked = window.filter(|cache| cache.committed_tail(&meta));
        if len != 0 && tracked.is_none() {
            let mut last = [0];
            if journal.read_exact_at(&mut last, len - 1).is_err() {
                return;
            }
            if last != [b'\n'] {
                let mut buf = [0u8; 4096];
                while len > 0 {
                    let count = len.min(buf.len() as u64) as usize;
                    len -= count as u64;
                    if journal.read_exact_at(&mut buf[..count], len).is_err() {
                        return;
                    }
                    if let Some(i) = memchr::memrchr(b'\n', &buf[..count]) {
                        len += i as u64 + 1;
                        break;
                    }
                }
                if journal.set_len(len).is_err() {
                    return;
                }
            }
        }
        if journal.write_all(entry.as_bytes()).is_err() {
            // Keep the next writer from joining its record to a partial write.
            let _ = journal.set_len(len);
        } else if let Some(cache) = tracked {
            cache.appended(&journal, entry.as_bytes());
        }
    }

    // Return false when input is waiting. The offset always points to the end
    // of the last complete record, so the next idle interval can resume there.
    pub fn ensure(
        &mut self,
        session: &Session,
        view: &mut View,
        all: bool,
        waiting: &dyn Fn() -> bool,
    ) -> bool {
        if *view == View::All || *view == View::Recent && !all {
            return true;
        }
        *view = View::Unread;
        if !self.refresh_unless(session, !all, waiting) {
            return false;
        }
        *view = if self.tail_start != 0 {
            View::Recent
        } else {
            View::All
        };
        true
    }

    fn reset_journal(&mut self, identity: Option<(u64, u64)>) {
        self.shared.clear();
        self.shared_query.clear();
        self.shared_match = None;
        self.displayed = None;
        self.journal = identity;
        self.tail_start = 0;
        self.older_prefix.clear();
        self.older_match = None;
        self.offset = 0;
    }

    fn refresh_unless(
        &mut self,
        session: &Session,
        recent_first: bool,
        input_ready: &dyn Fn() -> bool,
    ) -> bool {
        if input_ready() {
            return false;
        }
        let journal = file(session).and_then(|path| open_journal(&path).ok());
        let Some((mut journal, metadata)) = journal else {
            self.reset_journal(None);
            return true;
        };
        let identity = (metadata.dev(), metadata.ino());
        let snapshot = (metadata.len(), metadata.mtime(), metadata.mtime_nsec());
        let rewritten = snapshot.0 < self.snapshot.0
            || snapshot.0 == self.snapshot.0 && snapshot != self.snapshot;
        self.snapshot = snapshot;
        if self.journal != Some(identity)
            || rewritten
            || metadata.len() < self.offset
            || self.tail_start != 0 && !recent_first
        {
            self.reset_journal(Some(identity));
            // Recent matches are authoritative even before reading the older prefix.
            // Skip the first possibly split UTF-8 record, including its newline.
            const WINDOW: u64 = 16 * 1024;
            let end = metadata.len();
            if recent_first
                && end > 2 * WINDOW
                && journal.seek(SeekFrom::Start(end - WINDOW)).is_ok()
            {
                let mut tail = BufReader::new((&mut journal).take(WINDOW));
                if let Ok(n) = tail.skip_until(b'\n')
                    && (n as u64) < WINDOW
                    && n > 0
                {
                    self.offset = end - WINDOW + n as u64;
                    self.tail_start = self.offset;
                }
            }
        }
        if metadata.len() == self.offset || journal.seek(SeekFrom::Start(self.offset)).is_err() {
            return true;
        }
        // Writers append under an exclusive lock. Reading up to this snapshot
        // and accepting only newline-terminated rows also works mid-append.
        self.read_records(
            BufReader::new(journal.take(metadata.len() - self.offset)),
            input_ready,
        )
    }

    fn read_records(
        &mut self,
        mut reader: impl BufRead,
        mut input_ready: impl FnMut() -> bool,
    ) -> bool {
        self.shared_query.clear();
        let mut partial = Vec::new();
        loop {
            if input_ready() {
                return false;
            }
            let Ok(chunk) = reader.fill_buf() else {
                return true;
            };
            if chunk.is_empty() {
                return true;
            }
            for piece in chunk.split_inclusive(|&b| b == b'\n') {
                let Some(line) = piece.strip_suffix(b"\n") else {
                    partial.extend_from_slice(piece);
                    continue;
                };
                self.offset += (partial.len() + piece.len()) as u64;
                let row = if partial.is_empty() {
                    line
                } else {
                    partial.extend_from_slice(line);
                    &partial
                };
                if let Ok(row) = std::str::from_utf8(row)
                    && let Some(command) = command_field(row)
                {
                    self.shared.push(command);
                }
                partial.clear();
            }
            let length = chunk.len();
            reader.consume(length);
        }
    }

    // Search complete blocks backwards. Most blocks can be ruled out with
    // one byte substring search, without decoding or allocating any commands.
    fn older_match(
        &mut self,
        session: &Session,
        needle: &str,
        prefix: bool,
        mut skip: usize,
        waiting: &dyn Fn() -> bool,
    ) -> Result<Option<String>, ()> {
        const BLOCK: usize = 240 * 1024;
        let path = file(session).ok_or(())?;
        let (journal, m) = open_journal(&path).map_err(|_| ())?;
        if Some((m.dev(), m.ino())) != self.journal || m.len() < self.offset {
            return Err(());
        }
        let encoded = encoded_fragment(needle);
        let unicode = !encoded.is_ascii();
        let matches = journal_matcher(needle, prefix, &encoded);
        let mut end = self.tail_start;
        let cached = self.window.matching(&m);
        let block = &mut self.search_buffer;
        let mut width = BLOCK as u64;
        while end != 0 {
            if waiting() {
                return Err(());
            }
            let start = end.saturating_sub(width);
            let size = (end - start) as usize;
            let (buf, possible) =
                if let Some((bytes, pages)) = cached.and_then(|window| window.range(start, size)) {
                    (bytes, !unicode || pages.contains(&true))
                } else {
                    block.resize(size, 0);
                    journal.read_exact_at(block, start).map_err(|_| ())?;
                    (&block[..], true)
                };
            let boundary = if start == 0 {
                0
            } else {
                memchr::memchr(b'\n', buf).map_or(buf.len(), |index| index + 1)
            };
            if boundary == buf.len() {
                width = width.saturating_mul(2);
                continue;
            }
            width = BLOCK as u64;
            let rows = &buf[boundary..];
            if possible && let Some(hit) = matches(rows, &mut skip) {
                return Ok(Some(hit));
            }
            end = start + boundary as u64;
        }
        Ok(None)
    }

    pub fn older_suggestion<'a>(
        &'a mut self,
        session: &Session,
        view: &mut View,
        prefix: &str,
        waiting: &dyn Fn() -> bool,
    ) -> Option<&'a str> {
        if waiting() || *view != View::Recent {
            return None;
        }
        let reusable = !self.older_prefix.is_empty()
            && prefix.starts_with(&self.older_prefix)
            && self
                .older_match
                .as_ref()
                .is_none_or(|s| s.starts_with(prefix) && s.len() > prefix.len());
        if !reusable {
            if let Ok(result) = self.older_match(session, prefix, true, 0, waiting) {
                self.older_match = result;
                self.older_prefix.clear();
                self.older_prefix.push_str(prefix);
            } else {
                if !self.ensure(session, view, true, waiting) {
                    return None;
                }
                return self.suggestion(&[], prefix, true, || !waiting());
            }
        }
        self.displayed = Some(Hint::Older);
        self.older_match.as_deref().map(|s| &s[prefix.len()..])
    }

    pub fn find_command(
        &mut self,
        session: &Session,
        view: &mut View,
        queries: &[SearchQuery],
        waiting: &dyn Fn() -> bool,
    ) -> Result<Option<String>, ()> {
        if let Some((bytes, pages)) = self.window.complete(session) {
            let result = Self::best_match(queries, |needle, mut skip| {
                let encoded = encoded_fragment(needle);
                let matches = journal_matcher(needle, false, &encoded);
                let unicode = !encoded.is_ascii();
                let mut end = bytes.len();
                while end != 0 {
                    if waiting() {
                        return Err(());
                    }
                    let split = end.saturating_sub(240 * 1024);
                    let start = if split == 0 {
                        0
                    } else {
                        memchr::memrchr(b'\n', &bytes[..split]).map_or(0, |i| i + 1)
                    };
                    if (!unicode
                        || pages[start / window::PAGE..=(end - 1) / window::PAGE].contains(&true))
                        && let Some(hit) = matches(&bytes[start..end], &mut skip)
                    {
                        return Ok(Some(hit));
                    }
                    end = start;
                }
                Ok(None)
            })?;
            if result.is_some() || session.history_lines().is_empty() {
                return Ok(result);
            }
        }
        if !self.ensure(session, view, false, waiting) {
            return Err(());
        }
        if *view == View::Recent {
            if !self.shared.ends.is_empty() {
                // Refinements can all miss. Keep the raw reverse search available
                // for their shorter prefixes instead of decoding the entire journal.
                let result = Self::best_match(queries, |query, mut skip| {
                    if let Some(hit) = self.search_query(&[], query, &mut skip, waiting)? {
                        return Ok(Some(hit.to_owned()));
                    }
                    self.older_match(session, query, false, skip, waiting)
                });
                if result.is_ok() || waiting() {
                    return result;
                }
            }
            if !self.ensure(session, view, true, waiting) {
                return Err(());
            }
        }
        Self::best_match(queries, |query, mut skip| {
            Ok(self
                .search_query(session.history_lines(), query, &mut skip, waiting)?
                .map(str::to_owned))
        })
    }

    // A failed refinement keeps the newest match for the longest successful
    // prefix. Prefixes with the same skip count form a monotonic predicate.
    fn best_match(
        queries: &[SearchQuery],
        mut find: impl FnMut(&str, usize) -> Result<Option<String>, ()>,
    ) -> Result<Option<String>, ()> {
        for query in queries.iter().rev() {
            if let Some(found) = find(&query.text, query.skip)? {
                return Ok(Some(found));
            }
            if query.shortest < query.text.len() {
                let lengths: Vec<_> = query.text[query.shortest..]
                    .char_indices()
                    .map(|(offset, _)| query.shortest + offset)
                    .collect();
                let (mut lo, mut hi) = (0, lengths.len());
                let mut best = None;
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    if let Some(found) = find(&query.text[..lengths[mid]], query.skip)? {
                        best = Some(found);
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if best.is_some() {
                    return Ok(best);
                }
            }
        }
        Ok(None)
    }

    fn search_query<'a>(
        &'a self,
        local: &'a [String],
        query: &str,
        remaining: &mut usize,
        waiting: &dyn Fn() -> bool,
    ) -> Result<Option<&'a str>, ()> {
        let use_local = self.shared.ends.is_empty();
        let mut end = if use_local {
            local.len()
        } else {
            self.shared.ends.len()
        };
        while end > 0 {
            if waiting() {
                return Err(());
            }
            let start = end.saturating_sub(1024);
            for index in (start..end).rev() {
                let value = if use_local {
                    local[index].as_str()
                } else {
                    self.shared.get(index)
                };
                if value.contains(query) {
                    if *remaining == 0 {
                        return Ok(Some(value));
                    }
                    *remaining -= 1;
                }
            }
            end = start;
        }
        Ok(None)
    }

    pub fn displayed<'a>(&'a self, local: &'a [String]) -> Option<&'a str> {
        match self.displayed.as_ref()? {
            Hint::Local(i) => local.get(*i).map(String::as_str),
            Hint::Shared(i) if *i < self.shared.ends.len() => Some(self.shared.get(*i)),
            Hint::Shared(_) => None,
            Hint::Older => self.older_match.as_deref(),
        }
    }

    pub fn suggestion<'a>(
        &'a mut self,
        local: &'a [String],
        prefix: &str,
        shared: bool,
        mut may_scan: impl FnMut() -> bool,
    ) -> Option<&'a str> {
        self.displayed = None;
        if prefix.is_empty() {
            return None;
        }
        let suffix = |s: &str| s.len() > prefix.len() && s.starts_with(prefix);
        if let Some((last, earlier)) = local.split_last() {
            if suffix(last) {
                self.displayed = Some(Hint::Local(earlier.len()));
                return Some(&last[prefix.len()..]);
            }
            let mut end = earlier.len();
            for chunk in earlier.rchunks(1024) {
                if !may_scan() {
                    return None;
                }
                end -= chunk.len();
                if let Some(i) = chunk.iter().rposition(|s| suffix(s)) {
                    self.displayed = Some(Hint::Local(end + i));
                    return Some(&chunk[i][prefix.len()..]);
                }
            }
        }
        if !shared || self.shared.ends.is_empty() {
            return None;
        }
        let command = self.shared.get(self.shared.ends.len() - 1);
        if suffix(command) {
            self.displayed = Some(Hint::Shared(self.shared.ends.len() - 1));
            return Some(&command[prefix.len()..]);
        }
        let mut end = self.shared.ends.len();
        if !self.shared_query.is_empty() && prefix.starts_with(&self.shared_query) {
            // A longer query can only match at or before the previous match.
            let index = self.shared_match?;
            if suffix(self.shared.get(index)) {
                self.displayed = Some(Hint::Shared(index));
                return Some(&self.shared.get(index)[prefix.len()..]);
            }
            end = index;
        }
        let mut found = None;
        while end > 0 {
            if !may_scan() {
                return None;
            }
            let start = end.saturating_sub(4096);
            if let Some(i) = (start..end).rev().find(|&i| suffix(self.shared.get(i))) {
                found = Some(i);
                break;
            }
            end = start;
        }
        self.shared_query.clear();
        self.shared_query.push_str(prefix);
        self.shared_match = found;
        self.displayed = found.map(Hint::Shared);
        found.map(|i| &self.shared.get(i)[prefix.len()..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_builtin_history_keeps_complete_rows_and_resumes_after_rotation() {
        let root = std::env::temp_dir().join(format!("tonari-fc-journal-{}", std::process::id()));
        fs::create_dir_all(root.join("tonari")).unwrap();
        let mut session = Session::default();
        session.assign("HOME", root.to_str().unwrap());
        session.assign("XDG_STATE_HOME", root.to_str().unwrap());
        session.assign("HISTFILE", root.join(".bash_history").to_str().unwrap());
        session.assign("HISTSIZE", "3");
        let path = file(&session).unwrap();
        let rows = format!(
            "1\t1\t0\t/\t{}\n1\t1\t0\t/\tbefore\\n雪\n1\t1\t0\t/\tpartial",
            "a".repeat(17000)
        );
        fs::write(&path, rows).unwrap();
        let snapshot = InitialJournal::snapshot(&session).unwrap();
        session.push_history("local", true);
        snapshot.load(&mut session).unwrap();
        assert_eq!(session.history_lines()[1..], ["before\n雪", "local"]);
        assert_eq!(session.history.as_ref().unwrap().current, Some(2));
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"-done\n")
            .unwrap();
        let read_new = ["history".into(), "-n".into()];
        commands::run(&mut session, &read_new, 0);
        assert_eq!(
            session.history_lines(),
            ["before\n雪", "local", "partial-done"]
        );
        fs::rename(&path, path.with_extension("old")).unwrap();
        fs::write(&path, b"1\t1\t0\t/\treplaced\n").unwrap();
        commands::run(&mut session, &read_new, 0);
        assert_eq!(
            session.history_lines(),
            ["local", "partial-done", "replaced"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_hints_and_journal_appends_preserve_the_newest_match() {
        let mut history = History::default();
        history.read_records(
            b"1\t1\t0\t/tmp\tword one\n1\t1\t0\t/tmp\tword two\n".as_slice(),
            || false,
        );
        for i in 0..5000 {
            history.shared.push(&format!("padding {i}"));
        }
        let mut polls = 0;
        assert_eq!(
            history.suggestion(&[], "word", true, || {
                polls += 1;
                polls == 1
            }),
            None
        );
        assert_eq!(polls, 2);
        assert_eq!(history.suggestion(&[], "word", true, || true), Some(" two"));
        assert_eq!(
            history.suggestion(&[], "word t", true, || false),
            Some("wo")
        );
        assert_eq!(history.suggestion(&[], "word o", true, || true), Some("ne"));
        assert_eq!(history.suggestion(&[], "word z", true, || true), None);
        assert_eq!(
            history.suggestion(&[], "word zz", true, || panic!("known miss")),
            None
        );
        history.read_records(
            b"1\t2\t0\t/tmp\tword zzz\n1\t2\t0\t/tmp\tother\n".as_slice(),
            || false,
        );
        assert_eq!(history.suggestion(&[], "word zz", true, || true), Some("z"));
    }

    #[test]
    fn older_matches_skip_metadata_and_count_each_record_once() {
        let state =
            std::env::temp_dir().join(format!("tonari-history-test-{}", std::process::id()));
        fs::create_dir_all(state.join("tonari")).unwrap();
        let mut session = Session::default();
        session.assign("XDG_STATE_HOME", state.to_str().unwrap());
        session.assign("HOME", state.to_str().unwrap());
        session.assign("HISTFILE", state.join(".bash_history").to_str().unwrap());
        session.assign("HISTFILESIZE", "100000");
        let path = file(&session).unwrap();
        let mut rows = String::from("1\t1\t0\t/hit\thit oldest\n");
        // The oldest hit is outside the snapshot and the Unicode hit is inside it.
        for _ in 0..230000 {
            rows.push_str("1\t1\t0\t/hit\tpadding\n");
        }
        rows.push_str("1\t1\t0\t/hit\tcmd hit hit\\n雪\n1\t1\t0\t/hit\tfinal\\tword\n");
        fs::write(&path, &rows).unwrap();
        let m = fs::metadata(&path).unwrap();
        let mut history = History {
            journal: Some((m.dev(), m.ino())),
            offset: m.len(),
            tail_start: m.len(),
            ..History::default()
        };
        history.prepare_search(&session, &|| false);
        let mut search = |query, prefix, skip| {
            history
                .older_match(&session, query, prefix, skip, &|| false)
                .unwrap()
        };
        assert_eq!(search("hit", false, 0).as_deref(), Some("cmd hit hit\n雪"));
        assert_eq!(search("hit", false, 1).as_deref(), Some("hit oldest"));
        assert_eq!(search("hit", false, 2), None);
        assert_eq!(search("hit", true, 0).as_deref(), Some("hit oldest"));
        assert_eq!(
            search("hit\n雪", false, 0).as_deref(),
            Some("cmd hit hit\n雪")
        );
        assert_eq!(search("\t", false, 0).as_deref(), Some("final\tword"));
        assert_eq!(search("", false, 0).as_deref(), Some("final\tword"));
        fs::remove_file(path).unwrap();
        fs::remove_dir(state.join("tonari")).unwrap();
        fs::remove_dir(state).unwrap();
    }

    #[test]
    fn search_window_tracks_complete_commands_and_journal_replacement() {
        let root = std::env::temp_dir().join(format!("tonari-window-{}", std::process::id()));
        fs::create_dir_all(root.join("tonari")).unwrap();
        let mut session = Session::default();
        session.assign("HOME", root.to_str().unwrap());
        session.assign("HISTFILE", root.join(".bash_history").to_str().unwrap());
        session.assign("XDG_STATE_HOME", root.to_str().unwrap());
        let path = file(&session).unwrap();
        let pad = "1\t1\t0\t/needle\tcommon\n".repeat(2400);
        let first = format!("1\t1\t0\t/tmp\tfirst needle\n{pad}1\t1\t0\t/tmp\t雪 needle\n");
        fs::write(&path, first).unwrap();
        let mut history = History::default();
        history.prepare_search(&session, &|| false);
        let lookup = |h: &mut History, text: &str, skip| {
            h.find_command(
                &session,
                &mut View::Unread,
                &[SearchQuery {
                    text: text.into(),
                    skip,
                    shortest: text.len(),
                }],
                &|| false,
            )
            .unwrap()
        };
        assert_eq!(
            lookup(&mut history, "needle", 1).as_deref(),
            Some("first needle")
        );
        History::append(&session, "new 🌱", None, 0, Some(&mut history.window));
        assert!(history.window.complete(&session).is_some());
        assert_eq!(lookup(&mut history, "🌱", 0).as_deref(), Some("new 🌱"));
        let next = path.with_extension("next");
        fs::write(
            &next,
            format!("{pad}1\t2\t0\t/needle\tnew text\n1\t1\t0\t/\tincomplete 🌱"),
        )
        .unwrap();
        fs::rename(next, &path).unwrap();
        assert!(history.window.complete(&session).is_none());
        history.prepare_search(&session, &|| false);
        assert!(lookup(&mut history, "🌱", 0).is_none());
        assert!(lookup(&mut history, "needle", 0).is_none());
        assert_eq!(lookup(&mut history, "text", 0).as_deref(), Some("new text"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_read_restarts_at_the_first_unfinished_row() {
        let data = b"1\t1\t0\t/a\tfirst\n1\t2\t0\t/b\tsecond\\nline\n1\t3\t0\t/c\tlast\n";
        let mut history = History::default();
        // Stop after two short buffers, in the middle of the second record.
        let mut polls = 0;
        let input = BufReader::with_capacity(10, data.as_slice());
        assert!(!history.read_records(input, || {
            polls += 1;
            polls == 3
        }));
        assert_eq!(
            (0..history.shared.ends.len())
                .map(|i| history.shared.get(i))
                .collect::<Vec<_>>(),
            ["first"]
        );
        let offset = history.offset as usize;
        // The writer has yet to append the final newline.
        let partial = &data[offset..data.len() - 1];
        assert!(history.read_records(BufReader::with_capacity(3, partial), || false));
        assert_eq!(
            (0..history.shared.ends.len())
                .map(|i| history.shared.get(i))
                .collect::<Vec<_>>(),
            ["first", "second\nline"]
        );
        let rest = &data[history.offset as usize..];
        assert!(history.read_records(rest, || false));
        assert_eq!(
            (0..history.shared.ends.len())
                .map(|i| history.shared.get(i))
                .collect::<Vec<_>>(),
            ["first", "second\nline", "last"]
        );
        assert_eq!(history.offset, data.len() as u64);
    }
}
