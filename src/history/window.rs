// Bounded raw copy of the newest journal pages. External truncation or replacement
// cannot fault the editor while it is reading the copy.
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Copy, PartialEq)]
struct Stamp {
    identity: (u64, u64),
    len: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl From<&Metadata> for Stamp {
    fn from(m: &Metadata) -> Self {
        Self {
            identity: (m.dev(), m.ino()),
            len: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        }
    }
}

pub(super) const PAGE: usize = 16 * 1024;
const CAPACITY: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Window {
    stamp: Option<Stamp>,
    start: u64,
    bytes: Vec<u8>,
    unicode_pages: Vec<bool>,
    interrupted: bool,
}

impl Window {
    pub(super) fn prepare(&mut self, session: &crate::Session, waiting: &dyn Fn() -> bool) {
        let Some(path) = super::file(session) else {
            return;
        };
        let Ok((mut file, meta)) = super::open_journal(&path) else {
            return;
        };
        let stamp = Stamp::from(&meta);
        if self.stamp == Some(stamp) && !self.interrupted {
            return;
        }
        self.stamp = None;
        self.interrupted = false;
        self.bytes.clear();
        if stamp.len < 32 * 1024 || waiting() {
            return;
        }
        let count = stamp.len.min(CAPACITY as u64);
        self.start = stamp.len - count;
        if file.seek(SeekFrom::Start(self.start)).is_err() {
            return;
        }
        self.bytes.reserve_exact(count as usize);
        while self.bytes.len() < count as usize {
            if waiting() {
                self.interrupted = true;
                break;
            }
            let segment = (count - self.bytes.len() as u64).min(256 * 1024);
            if (&mut file).take(segment).read_to_end(&mut self.bytes).ok() != Some(segment as usize)
            {
                return;
            }
        }
        if !self.bytes.is_empty() {
            self.unicode_pages.clear();
            self.unicode_pages
                .extend(self.bytes.chunks(PAGE).map(|page| !page.is_ascii()));
            self.stamp = Some(stamp);
        }
    }

    pub(super) fn complete(&self, session: &crate::Session) -> Option<(&[u8], &[bool])> {
        let stamp = self.stamp?;
        if self.start != 0 || self.bytes.len() as u64 != stamp.len {
            return None;
        }
        let meta = std::fs::metadata(super::file(session)?).ok()?;
        if stamp != Stamp::from(&meta) {
            return None;
        }
        let end = memchr::memrchr(b'\n', &self.bytes).map_or(0, |i| i + 1);
        Some((&self.bytes[..end], &self.unicode_pages))
    }

    pub(super) fn matching(&self, meta: &Metadata) -> Option<&Self> {
        (self.stamp == Some(Stamp::from(meta))).then_some(self)
    }

    pub(super) fn range(&self, start: u64, size: usize) -> Option<&[u8]> {
        let offset = usize::try_from(start.checked_sub(self.start)?).ok()?;
        self.bytes.get(offset..)?.get(..size)
    }

    // A cooperating append under the journal lock leaves all copied bytes intact.
    pub(super) fn tracks(&self, file: &File) -> bool {
        self.stamp.is_some()
            && file
                .metadata()
                .is_ok_and(|m| self.stamp == Some(Stamp::from(&m)))
    }

    pub(super) fn appended(&mut self, file: &File, entry: &[u8]) {
        let old = self.stamp.take().unwrap();
        if let Ok(m) = file.metadata() {
            let new = Stamp::from(&m);
            if new.identity == old.identity
                && old.len.checked_add(entry.len() as u64) == Some(new.len)
                && self.start + self.bytes.len() as u64 == old.len
                && entry.len() <= CAPACITY - self.bytes.len()
            {
                let first_page = self.bytes.len() / PAGE;
                let needed = self.bytes.len() + entry.len();
                if needed > self.bytes.capacity() {
                    let capacity =
                        needed.max(self.bytes.capacity().saturating_mul(2).min(CAPACITY));
                    self.bytes.reserve_exact(capacity - self.bytes.len());
                }
                self.bytes.extend_from_slice(entry);
                self.unicode_pages.truncate(first_page);
                self.unicode_pages.extend(
                    self.bytes[first_page * PAGE..]
                        .chunks(PAGE)
                        .map(|p| !p.is_ascii()),
                );
                self.stamp = Some(new);
            }
        }
    }
}
