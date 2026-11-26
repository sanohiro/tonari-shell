use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

use crate::Session;

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

fn file(session: &Session) -> Option<PathBuf> {
    let setting = session.get("HISTFILE")?;
    if setting.is_empty()
        || setting == "/dev/null"
        || session.get("HISTFILESIZE").as_deref() == Some("0")
    {
        return None;
    }
    let dir = session
        .get("XDG_STATE_HOME")
        .filter(|v| PathBuf::from(v).is_absolute())
        .map(PathBuf::from)
        .or_else(|| {
            session
                .get("HOME")
                .map(|h| PathBuf::from(h).join(".local/state"))
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

fn command_field(row: &str) -> Option<&str> {
    journal_fields(row)?.0.splitn(4, '\t').nth(3)
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

#[derive(Clone, Default)]
pub(crate) struct Entries {
    pub lines: Vec<String>,
    timestamps: Vec<libc::time_t>,
    base: usize,
    current: Option<usize>,
    written: usize,
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
        let mut journal = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&self.path)?;
        let m = journal.metadata()?;
        if (m.dev(), m.ino()) != self.identity || m.len() < self.len || !m.is_file() {
            return Ok(());
        }
        let available = session
            .history_limit("HISTSIZE", 500)
            .saturating_sub(session.history_lines().len());
        let mut pos = self.len;
        let mut needed = available;
        let mut block = [0u8; 16384];
        if available > 0 {
            'scan: while pos > 0 {
                let count = pos.min(block.len() as u64) as usize;
                pos -= count as u64;
                journal.read_exact_at(&mut block[..count], pos)?;
                for index in memchr::memrchr_iter(b'\n', &block[..count]) {
                    if needed == 0 {
                        pos += index as u64 + 1;
                        break 'scan;
                    }
                    needed -= 1;
                }
            }
        }
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
            if let Some((body, ts)) = std::str::from_utf8(record).ok().and_then(journal_fields)
                && let Some(text) = body.splitn(4, '\t').nth(3).and_then(decode)
            {
                older.push(text);
                timestamps.push(ts.and_then(|s| s.parse().ok()).unwrap_or(UNKNOWN_TIME));
            }
            row.clear();
        }
        let h = session.history_mut();
        let n = older.len();
        older.append(&mut h.lines);
        h.lines = older;
        h.current = h.current.map(|index| index + n);
        timestamps.append(&mut h.timestamps);
        h.timestamps = timestamps;
        h.written += n;
        h.imported = Some(Self {
            len: complete,
            ..self.clone()
        });
        Ok(())
    }
}

impl Session {
    pub(crate) fn history_number(&self) -> usize {
        self.history
            .as_ref()
            .map_or(1, |h| h.base + h.lines.len() + 1)
    }
    pub(crate) fn history_lines(&self) -> &[String] {
        self.history.as_deref().map_or(&[], |h| h.lines.as_slice())
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
            if self
                .get("HOME")
                .is_some_and(|h| std::path::Path::new(&h).join(".bash_history") == path)
            {
                return;
            }
            if let Err(e) =
                commands::limit_plain_file(path, limit, self.get("HISTTIMEFORMAT").is_some())
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("tonari: {}: {e}", path.display());
            }
        }
    }

    fn history_limit(&self, key: &str, default: usize) -> usize {
        self.get(key)
            .and_then(|s| s.parse::<i64>().ok())
            .map_or(default, |n| usize::try_from(n).unwrap_or(usize::MAX))
    }
    pub(crate) fn begin_history_line(&mut self, text: &str) -> bool {
        if !self.history_enabled || !self.accept_history(text, self.history_lines().last()) {
            return false;
        }
        self.push_history(text.strip_suffix('\n').unwrap_or(text), true);
        true
    }
    fn push_history(&mut self, text: &str, current: bool) {
        let limit = self.history_limit("HISTSIZE", 500);
        if limit == 0 {
            return;
        }
        let erase = self
            .get("HISTCONTROL")
            .is_some_and(|v| v.split(':').any(|w| w == "erasedups"));
        let h = self.history_mut();
        if erase {
            for i in (0..h.lines.len()).rev() {
                if h.lines[i] == text {
                    h.remove(i);
                }
            }
        }
        h.lines.push(text.into());
        h.timestamps
            .push(unsafe { libc::time(std::ptr::null_mut()) });
        h.trim_to(limit);
        h.current = current.then(|| h.lines.len() - 1);
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
        let line = &h.lines[index];
        if self.history_enabled && self.accept_history(line, h.lines[..index].last()) {
            History::append(self, line, cwd, status, Some(&mut history.window));
        } else {
            self.history_mut().discard_current();
        }
    }

    fn accept_history(&self, source: &str, previous: Option<&String>) -> bool {
        let session = self;

        let text = source.strip_suffix('\n').unwrap_or(source);
        let limit = session
            .get("HISTSIZE")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(500);
        if limit == 0 || text.trim().is_empty() {
            return false;
        }
        let previous = previous.map(String::as_str);
        let control = session.get("HISTCONTROL").unwrap_or_default();
        for rule in control.split(':') {
            if matches!(rule, "ignorespace" | "ignoreboth") && text.starts_with(' ') {
                return false;
            }
            if matches!(rule, "ignoredups" | "ignoreboth") && previous == Some(text) {
                return false;
            }
        }
        if session.get("HISTIGNORE").is_some_and(|s| {
            s.split(':').any(|p| {
                !p.is_empty()
                    && (if p == "&" {
                        previous == Some(text)
                    } else {
                        crate::pattern::matches(text, p)
                    })
            })
        }) {
            return false;
        }
        true
    }
}

impl Entries {
    fn trim_to(&mut self, limit: usize) {
        if self.lines.len() > limit {
            let count = self.lines.len() - limit;
            self.lines.drain(..count);
            self.timestamps.drain(..count);
            self.base += count;
            self.written = self.written.saturating_sub(count);
            self.current = self.current.and_then(|index| index.checked_sub(count));
        }
    }

    fn remove(&mut self, index: usize) {
        self.lines.remove(index);
        self.timestamps.remove(index);
        if index < self.written {
            self.written -= 1;
        }
        self.current = self.current.and_then(|j| {
            if j == index {
                None
            } else {
                Some(j - usize::from(index < j))
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
            .create(true)
            .mode(0o600)
            .open(path)
        else {
            return;
        };
        if unsafe { libc::flock(journal.as_raw_fd(), libc::LOCK_EX) } != 0 {
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
        let tracked = window.filter(|cache| cache.tracks(&journal));
        if journal.write_all(entry.as_bytes()).is_ok()
            && let Some(cache) = tracked
        {
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

    fn refresh_unless(
        &mut self,
        session: &Session,
        recent_first: bool,
        input_ready: &dyn Fn() -> bool,
    ) -> bool {
        if input_ready() {
            return false;
        }
        let journal = file(session).and_then(|path| {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(path)
                .ok()?;
            let metadata = file.metadata().ok()?;
            metadata.is_file().then_some((file, metadata))
        });
        let Some((mut journal, metadata)) = journal else {
            self.shared.clear();
            self.journal = None;
            self.tail_start = 0;
            self.older_prefix.clear();
            self.older_match = None;
            self.offset = 0;
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
            self.shared.clear();
            self.journal = Some(identity);
            self.offset = 0;
            self.tail_start = 0;
            self.older_prefix.clear();
            self.older_match = None;
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
                if let Ok(row) = std::str::from_utf8(row) {
                    if let Some(command) = command_field(row) {
                        self.shared.push(command);
                    }
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
        let journal = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| ())?;
        let m = journal.metadata().map_err(|_| ())?;
        if Some((m.dev(), m.ino())) != self.journal || m.len() < self.offset {
            return Err(());
        }
        let encoded = encoded_fragment(needle);
        let matches = journal_matcher(needle, prefix, &encoded);
        let mut end = self.tail_start;
        let cached = self.window.bytes(&m);
        let block = &mut self.search_buffer;
        while end != 0 {
            if waiting() {
                return Err(());
            }
            let start = end.saturating_sub(BLOCK as u64);
            let size = (end - start) as usize;
            let buf = if let Some((base, data)) = cached
                && let Some(offset) = start.checked_sub(base)
                && let Some(bytes) = data.get(offset as usize..offset as usize + size)
            {
                bytes
            } else {
                block.resize(size, 0);
                journal.read_exact_at(block, start).map_err(|_| ())?;
                &block[..]
            };
            let boundary = if start == 0 {
                0
            } else {
                memchr::memchr(b'\n', buf).ok_or(())? + 1
            };
            if boundary == buf.len() {
                return Err(());
            } // An oversized record needs the streaming reader.
            let rows = &buf[boundary..];
            if let Some(hit) = matches(rows, &mut skip) {
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
        session.assign("HISTFILE", "history");
        session.assign("HISTFILESIZE", "100000");
        let path = file(&session).unwrap();
        let mut rows = String::from("1\t1\t0\t/hit\thit oldest\n");
        for _ in 0..42000 {
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
        session.assign("HISTFILE", "enabled");
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
