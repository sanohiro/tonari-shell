use super::{command_field, decode, encoded_fragment, file};
use crate::Session;
use foldhash::HashSet;
use memchr::{memchr, memmem, memrchr};
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::rc::Rc;

/// Spaces separate literal search fragments. An escaped space can itself
/// occur in a saved shell word, so keep both the backslash and the space.
pub(crate) fn fragments(mut query: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        let blank = |c: char| c.is_whitespace() && !c.is_control();
        query = query.trim_start_matches(blank);
        if query.is_empty() {
            return None;
        }
        let mut escaped = false;
        let end = query
            .char_indices()
            .find_map(|(i, c)| {
                let separator = !escaped && blank(c);
                escaped = !escaped && c == '\\';
                separator.then_some(i)
            })
            .unwrap_or(query.len());
        let (word, rest) = query.split_at(end);
        query = rest;
        Some(word)
    })
}

// Find the preceding row boundary and count metadata delimiters together.
// Work from the known end of a short header without decoding its timestamp,
// PID or directory. Tabs in the command itself must never alias a shorter hit.
fn duplicate_start(metadata: &[u8]) -> Option<usize> {
    fn matches(word: u64, byte: u8) -> u64 {
        const LOW: u64 = 0x7f7f_7f7f_7f7f_7f7f;
        let x = word ^ u64::from_ne_bytes([byte; 8]);
        !(((x & LOW) + LOW) | x | LOW)
    }
    let mut end = metadata.len();
    let mut tabs = 0;
    let start = loop {
        if end < 8 {
            let prefix = &metadata[..end];
            let start = prefix
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |i| i + 1);
            tabs += prefix[start..].iter().filter(|&&b| b == b'\t').count() as u32;
            break start;
        }
        let word = u64::from_le_bytes(metadata[end - 8..end].try_into().unwrap());
        let newlines = matches(word, b'\n');
        let delimiters = matches(word, b'\t');
        if newlines != 0 {
            let after = newlines.leading_zeros() / 8;
            tabs += (delimiters & !u64::MAX.checked_shr(after * 8).unwrap_or(0)).count_ones();
            break end - after as usize;
        }
        tabs += delimiters.count_ones();
        if tabs > 4 {
            return None;
        }
        end -= 8;
    };
    (metadata
        .get(start..start + 2)
        .is_some_and(|v| matches!((v, tabs), (b"1\t", 3) | (b"2\t", 4))))
    .then_some(start)
}

// One picker owns a snapshot of the journal length. New pages resume from the
// last examined record. A new query rewinds offsets and reuses the read buffer.
pub(crate) struct Filter<'a> {
    cached: Option<&'a super::window::Window>,
    file: Option<File>,
    length: u64,
    end: u64,
    local_end: usize,
    buf: Vec<u8>,
    pub hits: Vec<Rc<str>>,
    seen: Option<HashSet<Rc<str>>>,
}

impl<'a> Filter<'a> {
    pub fn new(session: &Session, history: &'a super::History) -> Self {
        let journal = file(session).and_then(|p| {
            let (f, m) = super::open_journal(&p).ok()?;
            Some((f, m.len(), history.window.matching(&m)))
        });
        let (file, length, cached) =
            journal.map_or((None, 0, None), |(f, size, cached)| (Some(f), size, cached));
        Self {
            cached,
            file,
            length,
            end: length,
            local_end: session.history_lines().len(),
            buf: Vec::new(),
            hits: Vec::new(),
            seen: None,
        }
    }

    pub fn rewind(&mut self, session: &Session) {
        self.hits.clear();
        self.seen = None;
        self.end = self.length;
        self.local_end = session.history_lines().len();
    }

    pub fn exhausted(&self) -> bool {
        self.end == 0 && self.local_end == 0
    }

    pub fn fill(
        &mut self,
        session: &Session,
        query: &str,
        wanted: usize,
        waiting: impl Fn() -> bool,
    ) {
        if self.hits.len() >= wanted || self.exhausted() {
            return;
        }
        let terms: Vec<_> = fragments(query).collect();
        let matches = |line: &str| terms.iter().all(|t| line.contains(t));
        let mut record = |line: &str| {
            if !matches(line) {
                return false;
            }
            // A few visible pages fit in a short linear scan. Deeper pages
            // share string storage with an index for both journal sources.
            const INDEX_AFTER: usize = 32;
            if self.hits.len() == INDEX_AFTER && self.seen.is_none() {
                self.seen = Some(self.hits.iter().cloned().collect());
            }
            let known = self.seen.as_ref().map_or_else(
                || self.hits.iter().any(|s| s.as_ref() == line),
                |seen| seen.contains(line),
            );
            if known {
                return false;
            }
            let line: Rc<str> = Rc::from(line);
            if let Some(seen) = &mut self.seen {
                seen.insert(Rc::clone(&line));
            }
            self.hits.push(line);
            self.hits.len() == wanted
        };
        while self.local_end > 0 {
            if self.local_end.is_multiple_of(256) && waiting() {
                return;
            }
            self.local_end -= 1;
            let line = &session.history_lines()[self.local_end];
            if record(line) {
                return;
            }
        }

        let Some(file) = &self.file else { return };
        let raw: Vec<_> = terms.iter().map(|t| encoded_fragment(t)).collect();
        let anchor = raw
            .iter()
            .max_by_key(|s| s.len())
            .map_or("", |s| s.as_ref());
        let non_ascii = anchor.bytes().find(|b| !b.is_ascii());
        let reverse = memmem::FinderRev::new(anchor.as_bytes());
        let needs_unicode = raw.iter().any(|s| !s.is_ascii());
        const BLOCK: usize = 128 * 1024;
        let mut width = BLOCK;
        while self.end > 0 {
            if waiting() {
                return;
            }
            let start = self.end.saturating_sub(width as u64);
            let size = (self.end - start) as usize;
            let mut ascii_only = false;
            let buf = if let Some((bytes, pages)) =
                self.cached.and_then(|window| window.range(start, size))
            {
                ascii_only = needs_unicode && pages.iter().all(|&v| !v);
                bytes
            } else {
                self.buf.resize(size, 0);
                for (i, part) in self.buf.chunks_mut(BLOCK).enumerate() {
                    if i != 0 && waiting() {
                        return;
                    }
                    if file
                        .read_exact_at(part, start + (i * BLOCK) as u64)
                        .is_err()
                    {
                        self.end = 0;
                        return;
                    }
                }
                &self.buf
            };
            let boundary = if start == 0 {
                0
            } else {
                match memchr(b'\n', buf) {
                    Some(i) if i + 1 < size => i + 1,
                    _ => {
                        // An unusually long command crosses a block. Grow until
                        // its start is available, keeping input checks bounded.
                        width = width.saturating_mul(2);
                        continue;
                    }
                }
            };
            width = BLOCK;
            self.end = start + boundary as u64;
            if ascii_only {
                continue;
            }
            let block = &buf[boundary..];
            let Some(end) = memrchr(b'\n', block) else {
                continue;
            };
            let mut rows = &block[..end]; // ignore the unfinished journal tail
            if non_ascii.is_some_and(|b| memchr(b, rows).is_none())
                || !raw.iter().all(|term| {
                    term.as_ref() == anchor || memmem::find(rows, term.as_bytes()).is_some()
                })
            {
                continue;
            }
            let base = self.end;
            let mut checked = 0usize;
            let mut recent = ["", ""];
            while !rows.is_empty() {
                if checked.is_multiple_of(256) && waiting() {
                    self.end = base + rows.len() as u64 + 1;
                    return;
                }
                checked += 1;
                // Duplicate commands also occur with different timestamps, PIDs and
                // directories. Verify the delimiter count so a suffix of a command
                // containing literal tabs cannot hide a distinct entry.
                if let Some(metadata) = recent
                    .iter()
                    .filter(|cmd| !cmd.is_empty())
                    .find_map(|cmd| rows.strip_suffix(cmd.as_bytes())?.strip_suffix(b"\t"))
                    && let Some(start) = duplicate_start(metadata)
                {
                    rows = &rows[..start.saturating_sub(1)];
                    continue;
                }
                let pos = if anchor.is_empty() {
                    rows.len()
                } else {
                    let Some(hit) = reverse.rfind(rows) else {
                        break;
                    };
                    hit
                };
                let begin = memrchr(b'\n', &rows[..pos]).map_or(0, |i| i + 1);
                let end = memchr(b'\n', &rows[pos..]).map_or(rows.len(), |i| pos + i);
                let row = &rows[begin..end];
                rows = &rows[..begin.saturating_sub(1)];
                let Some(field) = std::str::from_utf8(row).ok().and_then(command_field) else {
                    continue;
                };
                // Consecutive identical records are common after journal imports.
                // Compare complete records, doubling verified spans within this block.
                let mut span = row.len() + 1;
                let repeated = |end: usize, n: usize| {
                    end >= n && block.get(end..end + n) == Some(&block[end - n..end])
                };
                while repeated(rows.len(), span) {
                    rows = &rows[..rows.len() - span];
                    span *= 2;
                }
                while span > row.len() + 1 {
                    span /= 2;
                    if repeated(rows.len(), span) {
                        rows = &rows[..rows.len() - span];
                    }
                }
                if recent.contains(&field) {
                    continue;
                }
                recent = [field, recent[0]];
                let decoded;
                let line = if field.contains('\\') {
                    let Some(value) = decode(field) else { continue };
                    decoded = value;
                    &decoded
                } else {
                    field
                };
                if record(line) {
                    self.end = base + begin as u64;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::duplicate_start;

    #[test]
    fn duplicate_headers_require_the_exact_number_of_fields() {
        for offset in 0..32 {
            for header in [
                "1\t7\t0\t/dir",
                "2\t100\t7\t0\t/dir",
                "1\t7\t0\t/dir\textra",
                "0\t7\t0\t/dir",
                "1\t0\t/dir",
            ] {
                let expected = matches!(header, "1\t7\t0\t/dir" | "2\t100\t7\t0\t/dir");
                let input = format!("{}\n{header}", "x".repeat(offset));
                assert_eq!(
                    duplicate_start(input.as_bytes()),
                    expected.then_some(offset + 1)
                );
                assert_eq!(duplicate_start(header.as_bytes()), expected.then_some(0));
            }
        }
    }
}
