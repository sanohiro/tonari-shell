use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

use crate::Session;

#[derive(Default)]
pub struct History {
    shared: JournalEntries,
    shared_query: String,
    shared_match: Option<usize>,
    journal: Option<(u64, u64)>,
    snapshot: (u64, i64, i64),
    offset: u64,
    tail_start: u64,
    older_prefix: String,
    older_match: Option<String>,
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

fn command_field(row: &str) -> Option<&str> {
    row.strip_prefix("1\t")?.splitn(4, '\t').nth(3)
}

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

pub(crate) mod commands;
pub(crate) mod expand;

#[derive(Clone, Default)]
pub(crate) struct Entries {
    pub lines: Vec<String>,
    timestamps: Vec<libc::time_t>,
    base: usize,
    current: Option<usize>,
    written: usize,
    imported: Option<(PathBuf, usize)>,
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
        if h.lines.len() > limit {
            let count = h.lines.len() - limit;
            h.lines.drain(..count);
            h.timestamps.drain(..count);
            h.base += count;
            h.written = h.written.saturating_sub(count);
        }
        h.current = current.then(|| h.lines.len() - 1);
    }
    pub(crate) fn finish_history_line(&mut self) {
        if let Some(data) = &mut self.history {
            std::rc::Rc::make_mut(data).current = None;
        }
    }

    pub(crate) fn persist_current_history(&mut self, cwd: Option<&str>, status: u8) {
        let Some(h) = self.history.as_deref() else {
            return;
        };
        let Some(index) = h.current else { return };
        let line = &h.lines[index];
        if self.history_enabled && self.accept_history(line, h.lines[..index].last()) {
            History::persist(self, line, cwd, status);
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

impl History {
    pub fn persist(session: &Session, source: &str, cwd: Option<&str>, status: u8) {
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
        let mut entry = format!("1\t{}\t{status}\t", std::process::id());
        encode(cwd.unwrap_or(""), &mut entry);
        entry.push('\t');
        encode(text, &mut entry);
        entry.push('\n');
        let _ = journal.write_all(entry.as_bytes());
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
        &self,
        session: &Session,
        needle: &str,
        prefix: bool,
        mut skip: usize,
        waiting: &dyn Fn() -> bool,
    ) -> Result<Option<String>, ()> {
        const BLOCK: usize = 128 * 1024;
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
        // A tab or CR can also occur literally in the command field of an
        // existing journal. Either spelling still contains this encoded part.
        let part = needle.split(['\t', '\r']).max_by_key(|s| s.len()).unwrap();
        let at_start = prefix && part.len() == needle.len();
        let mut encoded = String::new();
        let raw = if part.contains(['\\', '\n']) {
            encode(part, &mut encoded);
            encoded.as_str()
        } else {
            part
        };
        let finder = memchr::memmem::Finder::new(raw.as_bytes());
        let mut end = self.tail_start;
        let mut block = vec![0; BLOCK.min(end as usize)];
        while end != 0 {
            if waiting() {
                return Err(());
            }
            let start = end.saturating_sub(BLOCK as u64);
            let buf = &mut block[..(end - start) as usize];
            journal.read_exact_at(buf, start).map_err(|_| ())?;
            let boundary = if start == 0 {
                0
            } else {
                memchr::memchr(b'\n', buf).ok_or(())? + 1
            };
            if boundary == buf.len() {
                return Err(());
            } // An oversized record needs the streaming reader.
            let rows = &buf[boundary..];
            // Only candidate blocks need record parsing and UTF-8 validation.
            if finder.find(rows).is_some() {
                for line in rows.rsplit(|&b| b == b'\n') {
                    if let Ok(line) = std::str::from_utf8(line)
                        && let Some(word) = command_field(line)
                        && if at_start {
                            word.starts_with(raw)
                        } else {
                            word.contains(raw)
                        }
                        && let Some(command) = decode(word)
                        && if prefix {
                            command.len() > needle.len() && command.starts_with(needle)
                        } else {
                            command.contains(needle)
                        }
                    {
                        if skip == 0 {
                            return Ok(Some(command));
                        }
                        skip -= 1;
                    }
                }
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
        self.older_match.as_deref().map(|s| &s[prefix.len()..])
    }

    pub fn find_command(
        &mut self,
        session: &Session,
        view: &mut View,
        queries: &[(String, usize)],
        waiting: &dyn Fn() -> bool,
    ) -> Result<Option<String>, ()> {
        if !self.ensure(session, view, false, waiting) {
            return Err(());
        }
        if *view == View::Recent {
            let (query, skip) = queries.last().unwrap();
            let mut remaining = *skip;
            if let Some(found) = self.search_query(&[], query, &mut remaining, waiting)? {
                return Ok(Some(found.to_owned()));
            }
            if !waiting()
                && let Ok(found) = self.older_match(session, query, false, remaining, waiting)
                && (found.is_some() || queries.len() == 1 && !self.shared.ends.is_empty())
            {
                return Ok(found);
            }
            if !self.ensure(session, view, true, waiting) {
                return Err(());
            }
        }
        // A failed refinement keeps the last successful result, even if keys
        // arrived before the previous query had completed.
        for (query, skip) in queries.iter().rev() {
            let mut remaining = *skip;
            if let Some(found) =
                self.search_query(session.history_lines(), query, &mut remaining, waiting)?
            {
                return Ok(Some(found.to_owned()));
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
                return Some(&last[prefix.len()..]);
            }
            for chunk in earlier.rchunks(1024) {
                if !may_scan() {
                    return None;
                }
                if let Some(command) = chunk.iter().rev().find(|s| suffix(s.as_str())) {
                    return Some(&command[prefix.len()..]);
                }
            }
        }
        if !shared || self.shared.ends.is_empty() {
            return None;
        }
        let command = self.shared.get(self.shared.ends.len() - 1);
        if suffix(command) {
            return Some(&command[prefix.len()..]);
        }
        let mut end = self.shared.ends.len();
        if !self.shared_query.is_empty() && prefix.starts_with(&self.shared_query) {
            // A longer query can only match at or before the previous match.
            let index = self.shared_match?;
            if suffix(self.shared.get(index)) {
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
        found.map(|i| &self.shared.get(i)[prefix.len()..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
