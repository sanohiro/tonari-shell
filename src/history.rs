use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

use crate::Session;

#[derive(Default)]
pub struct History {
    pub local: Vec<String>,
    shared: Vec<String>,
    shared_query: String,
    shared_match: Option<usize>,
    journal: Option<(u64, u64)>,
    offset: u64,
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

fn decode(value: &str) -> Option<String> {
    if !value.as_bytes().contains(&b'\\') {
        return Some(value.to_owned());
    }
    let mut chars = value.chars();
    let mut text = String::new();
    while let Some(c) = chars.next() {
        text.push(if c == '\\' {
            match chars.next()? {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '\\' => '\\',
                _ => return None,
            }
        } else {
            c
        });
    }
    Some(text)
}

impl History {
    pub fn accepts(&self, session: &Session, source: &str) -> bool {
        let text = source.strip_suffix('\n').unwrap_or(source);
        let limit = session
            .get("HISTSIZE")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(500);
        if limit == 0 || text.trim().is_empty() {
            return false;
        }
        let previous = self.local.last().map(String::as_str);
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

    pub fn record(&mut self, session: &Session, source: &str, cwd: Option<&str>, status: u8) {
        let text = source.strip_suffix('\n').unwrap_or(source);
        let control = session.get("HISTCONTROL").unwrap_or_default();
        let limit = session
            .get("HISTSIZE")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(500);
        if control.split(':').any(|v| v == "erasedups") {
            self.local.retain(|v| v != text);
        }
        self.local.push(text.to_owned());
        if limit > 0 && self.local.len() > limit as usize {
            self.local.drain(..self.local.len() - limit as usize);
        }
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
    pub fn refresh_unless(
        &mut self,
        session: &Session,
        mut input_ready: impl FnMut() -> bool,
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
            self.offset = 0;
            return true;
        };
        let identity = (metadata.dev(), metadata.ino());
        if self.journal != Some(identity) || metadata.len() < self.offset {
            self.shared.clear();
            self.journal = Some(identity);
            self.offset = 0;
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
                    if let Some(command) = row
                        .strip_prefix("1\t")
                        .and_then(|fields| fields.splitn(4, '\t').nth(3))
                        .and_then(decode)
                    {
                        self.shared.push(command);
                    }
                }
                partial.clear();
            }
            let length = chunk.len();
            reader.consume(length);
        }
    }

    pub fn search(&self, query: &str, skip: usize) -> Option<&str> {
        let entries = if self.shared.is_empty() {
            &self.local
        } else {
            &self.shared
        };
        entries
            .iter()
            .rev()
            .filter(|v| v.contains(query))
            .nth(skip)
            .map(String::as_str)
    }

    pub fn suggestion(
        &mut self,
        prefix: &str,
        shared: bool,
        mut may_scan: impl FnMut() -> bool,
    ) -> Option<&str> {
        if prefix.is_empty() {
            return None;
        }
        let suffix = |s: &'_ String| s.len() > prefix.len() && s.starts_with(prefix);
        if let Some(command) = self.local.iter().rev().find(|s| suffix(s)) {
            return Some(&command[prefix.len()..]);
        }
        if !shared || self.shared.is_empty() {
            return None;
        }
        if let Some(command) = self.shared.last().filter(|s| suffix(s)) {
            return Some(&command[prefix.len()..]);
        }
        let mut end = self.shared.len();
        if !self.shared_query.is_empty() && prefix.starts_with(&self.shared_query) {
            // A longer query can only match at or before the previous match.
            let index = self.shared_match?;
            if suffix(&self.shared[index]) {
                return Some(&self.shared[index][prefix.len()..]);
            }
            end = index;
        }
        let mut found = None;
        while end > 0 {
            if !may_scan() {
                return None;
            }
            let start = end.saturating_sub(4096);
            if let Some(offset) = self.shared[start..end].iter().rposition(suffix) {
                found = Some(start + offset);
                break;
            }
            end = start;
        }
        self.shared_query.clear();
        self.shared_query.push_str(prefix);
        self.shared_match = found;
        found.map(|i| &self.shared[i][prefix.len()..])
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
        history
            .shared
            .extend((0..5000).map(|n| format!("padding {n}")));
        let mut polls = 0;
        assert_eq!(
            history.suggestion("word", true, || {
                polls += 1;
                polls == 1
            }),
            None
        );
        assert_eq!(polls, 2);
        assert_eq!(history.suggestion("word", true, || true), Some(" two"));
        assert_eq!(history.suggestion("word t", true, || false), Some("wo"));
        assert_eq!(history.suggestion("word o", true, || true), Some("ne"));
        assert_eq!(history.suggestion("word z", true, || true), None);
        assert_eq!(
            history.suggestion("word zz", true, || panic!("known miss")),
            None
        );
        history.read_records(
            b"1\t2\t0\t/tmp\tword zzz\n1\t2\t0\t/tmp\tother\n".as_slice(),
            || false,
        );
        assert_eq!(history.suggestion("word zz", true, || true), Some("z"));
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
        assert_eq!(history.shared, ["first"]);
        let offset = history.offset as usize;
        // The writer has yet to append the final newline.
        let partial = &data[offset..data.len() - 1];
        assert!(history.read_records(BufReader::with_capacity(3, partial), || false));
        assert_eq!(history.shared, ["first", "second\nline"]);
        let rest = &data[history.offset as usize..];
        assert!(history.read_records(rest, || false));
        assert_eq!(history.shared, ["first", "second\nline", "last"]);
        assert_eq!(history.offset, data.len() as u64);
    }
}
