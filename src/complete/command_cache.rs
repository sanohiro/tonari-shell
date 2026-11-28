use crate::pattern::directory;
use std::cell::RefCell;
use std::ffi::CStr;
use std::ops::ControlFlow;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) const MAX_DIRECTORIES: usize = 32;
const MAX_ENTRIES: usize = 8192;
const MAX_NAME_BYTES: usize = 256 * 1024;

#[derive(PartialEq)]
struct Stamp {
    identity: (u64, u64),
    modified: (i64, i64),
    changed: (i64, i64),
}

struct Listing {
    heads: [u16; 256],
    bytes: Vec<u8>,
    index: Vec<Entry>,
}

struct Entry {
    start: u32,
    len: u16,
    kind: u8,
    next: u16,
}

impl Entry {
    fn name<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.start as usize..self.start as usize + usize::from(self.len)]
    }
}

impl Listing {
    fn visit_prefix(
        &self,
        prefix: &[u8],
        visit: &mut impl FnMut(Option<&CStr>, u8) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let mut all = 0;
        let mut link = prefix.first().map_or(0, |&first| self.heads[usize::from(first)]);
        let editing = crate::editor::preview::active();
        let mut visited = 0usize;
        while link != 0 || prefix.is_empty() && all < self.index.len() {
            let entry = if prefix.is_empty() {
                let index = all; all += 1; &self.index[index]
            } else { &self.index[usize::from(link) - 1] };
            link = entry.next;
            if editing && visited.is_multiple_of(256) {
                crate::editor::preview::scan_input();
            }
            visited += 1;
            let bytes = entry.name(&self.bytes);
            if !bytes.starts_with(prefix) {
                continue;
            }
            // SAFETY: push copies the complete CStr including its terminating NUL,
            // and records that exact range in the append-only byte buffer.
            let name = unsafe { CStr::from_bytes_with_nul_unchecked(bytes) };
            // A regular name from a cached listing may have become a directory
            // within one filesystem timestamp tick. Recheck its type on lookup.
            let kind = if cfg!(target_os = "linux") && entry.kind == libc::DT_REG {
                libc::DT_UNKNOWN
            } else {
                entry.kind
            };
            visit(Some(name), kind)?;
        }
        ControlFlow::Continue(())
    }

    fn push(&mut self, name: &CStr, kind: u8) -> bool {
        let bytes = name.to_bytes_with_nul();
        let Ok(len) = u16::try_from(bytes.len()) else {
            return false;
        };
        if self.index.len() == MAX_ENTRIES || self.bytes.len() + bytes.len() > MAX_NAME_BYTES {
            return false;
        }
        // One-based links reserve zero for the end of a first-byte bucket.
        let head = &mut self.heads[usize::from(bytes[0])];
        self.index.push(Entry {
            start: self.bytes.len() as u32,
            len,
            kind,
            next: *head,
        });
        *head = self.index.len() as u16;
        self.bytes.extend_from_slice(bytes);
        true
    }
}

#[derive(Default)]
struct Cache {
    directories: Vec<(PathBuf, Stamp, Option<Listing>)>,
    next: usize,
}

thread_local! { static CACHE: RefCell<Cache> = RefCell::default(); }

pub(super) fn scan(
    path: &Path,
    prefix: &[u8],
    checked_fd: &mut bool,
    visit: &mut impl FnMut(Option<&CStr>, u8) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let Ok(meta) = path.metadata() else {
        return ControlFlow::Continue(());
    };
    let stamp = Stamp {
        identity: (meta.dev(), meta.ino()),
        modified: (meta.mtime(), meta.mtime_nsec()),
        changed: (meta.ctime(), meta.ctime_nsec()),
    };
    CACHE.with_borrow_mut(|cache| {
        let slot = cache.directories.iter().position(|(dir, _, _)| dir == path);
        if let Some(i) = slot {
            let (_, old, listing) = &cache.directories[i];
            if *old == stamp {
                let Some(listing) = listing else {
                    return directory::scan(path, prefix, visit);
                };
                // The stamp includes directory permission changes. Verify that
                // the process can still open a directory under its current fd limit.
                if !*checked_fd {
                    if std::fs::File::options()
                        .read(true)
                        .custom_flags(libc::O_DIRECTORY)
                        .open(path)
                        .is_err()
                    {
                        return ControlFlow::Continue(());
                    }
                    *checked_fd = true;
                }
                return listing.visit_prefix(prefix, visit);
            }
            cache.directories.swap_remove(i);
        }

        // Directory sizes are filesystem-specific. Bound this initial capacity hint.
        let entries = (meta.len() / 32).min(256) as usize;
        let mut listing = Some(Listing {
            heads: [0; 256],
            bytes: Vec::with_capacity(entries * 16),
            index: Vec::with_capacity(entries),
        });
        let result = directory::scan_checked(path, b"", &mut |raw, kind| {
            if let (Some(name), Some(active)) = (raw, &mut listing)
                && !active.push(name, kind)
            {
                listing = None;
            }
            // Untouched entries still poll for typeahead. An ambiguous prefix
            // can stop the live scan without publishing a partial index.
            visit(raw.filter(|s| s.to_bytes().starts_with(prefix)), kind)
        });
        if result == Some(ControlFlow::Continue(())) {
            // Remember an oversized directory so the next query can stream it.
            let entry = (path.to_owned(), stamp, listing);
            if cache.directories.len() < MAX_DIRECTORIES {
                cache.directories.push(entry);
            } else {
                cache.directories[cache.next] = entry;
                cache.next = (cache.next + 1) % MAX_DIRECTORIES;
            }
        }
        result.unwrap_or(ControlFlow::Continue(()))
    })
}
