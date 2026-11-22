use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

use crate::Session;

#[derive(Default)]
pub struct History {
    pub local: Vec<String>,
    shared: Vec<String>,
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

    pub fn refresh(&mut self, session: &Session) {
        let Some(path) = file(session) else {
            self.shared.clear();
            self.journal = None;
            self.offset = 0;
            return;
        };
        let Ok(mut journal) = fs::File::open(path) else {
            self.shared.clear();
            self.journal = None;
            self.offset = 0;
            return;
        };
        if unsafe { libc::flock(journal.as_raw_fd(), libc::LOCK_SH) } != 0 {
            return;
        }
        let Ok(metadata) = journal.metadata() else {
            return;
        };
        let identity = (metadata.dev(), metadata.ino());
        if self.journal != Some(identity) || metadata.len() < self.offset {
            self.shared.clear();
            self.journal = Some(identity);
            self.offset = 0;
        }
        if metadata.len() == self.offset || journal.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut reader = BufReader::new(journal);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            if reader.read_until(b'\n', &mut bytes).is_err() || !bytes.ends_with(b"\n") {
                return;
            }
            self.offset += bytes.len() as u64;
            let Ok(line) = std::str::from_utf8(&bytes[..bytes.len() - 1]) else {
                continue;
            };
            let mut fields = line.splitn(5, '\t');
            if fields.next() == Some("1") {
                if let Some(command) = fields.nth(3).and_then(decode) {
                    self.shared.push(command);
                }
            }
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

    pub fn suggestion(&self, prefix: &str) -> Option<&str> {
        if prefix.is_empty() {
            return None;
        }
        self.local
            .iter()
            .rev()
            .chain(self.shared.iter().rev())
            .find_map(|s| s.strip_prefix(prefix).filter(|s| !s.is_empty()))
    }
}
