use std::cell::RefCell;
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Collation {
    Bytes,
    FixedLocale,
    Current,
}

// Keep the enumeration order as NUL-delimited pathnames and the permutation
// that sorted them. Every expansion still reads and matches the live directory.
struct PreviousOrder {
    collation: Collation,
    names: Vec<u8>,
    indices: Vec<usize>,
}

thread_local! {
    static PREVIOUS: RefCell<PreviousOrder> = const { RefCell::new(PreviousOrder {
        collation: Collation::Current, names: Vec::new(), indices: Vec::new(),
    }) };
}

impl PreviousOrder {
    fn matches(&self, paths: &[String], collation: Collation) -> bool {
        if self.collation != collation || self.indices.len() != paths.len() {
            return false;
        }
        let mut rest = self.names.as_slice();
        for path in paths {
            let Some(tail) = rest
                .strip_prefix(path.as_bytes())
                .and_then(|s| s.strip_prefix(&[0]))
            else {
                return false;
            };
            rest = tail;
        }
        rest.is_empty()
    }

    fn update(&mut self, paths: &mut [String], collation: Collation, bytes: usize) {
        self.collation = collation;
        self.names.clear();
        self.names.reserve_exact(bytes);
        for path in paths.iter() {
            self.names.extend_from_slice(path.as_bytes());
            self.names.push(0);
        }
        sort_indices(paths, collation, bytes, &mut self.indices);
    }
}

fn sort_indices(
    paths: &mut [String],
    collation: Collation,
    _path_bytes: usize,
    indices: &mut Vec<usize>,
) {
    indices.clear();
    indices.reserve_exact(paths.len());
    indices.extend(0..paths.len());
    if collation == Collation::Bytes {
        indices.sort_unstable_by(|&i, &j| paths[i].cmp(&paths[j]));
    } else {
        #[cfg(target_os = "macos")]
        if sort_utf8(paths, _path_bytes, indices) {
            return;
        }
        for path in paths.iter_mut() {
            path.push('\0');
        }
        indices.sort_unstable_by(|&i, &j| unsafe { compare_terminated(&paths[i], &paths[j]) });
        for path in paths {
            path.pop();
        }
    }
}

#[cfg(target_os = "macos")]
fn sort_utf8(paths: &[String], bytes: usize, indices: &mut [usize]) -> bool {
    use std::ffi::CStr;
    unsafe extern "C" {
        fn wcscoll(a: *const libc::wchar_t, b: *const libc::wchar_t) -> libc::c_int;
    }
    let codeset = unsafe { libc::nl_langinfo(libc::CODESET) };
    if codeset.is_null() || unsafe { CStr::from_ptr(codeset) }.to_bytes() != b"UTF-8" {
        return false;
    }
    // Darwin strcoll converts both operands to wide strings before wcscoll.
    // In a UTF-8 locale each Rust scalar is exactly one Darwin wchar_t.
    let mut wide = Vec::new();
    let mut offsets = Vec::new();
    if wide.try_reserve_exact(bytes).is_err() || offsets.try_reserve_exact(paths.len()).is_err() {
        return false;
    }
    for path in paths {
        offsets.push(wide.len());
        wide.extend(path.chars().map(|c| c as libc::wchar_t));
        wide.push(0);
    }
    indices.sort_unstable_by(|&i, &j| {
        let order = unsafe { wcscoll(wide[offsets[i]..].as_ptr(), wide[offsets[j]..].as_ptr()) };
        order.cmp(&0).then_with(|| paths[i].cmp(&paths[j]))
    });
    true
}

// Safety: both paths must have a trailing NUL for the duration of the call.
unsafe fn compare_terminated(a: &str, b: &str) -> Ordering {
    let order = unsafe { libc::strcoll(a.as_ptr().cast(), b.as_ptr().cast()) };
    order.cmp(&0).then_with(|| a.cmp(b))
}

pub(super) fn append(mut paths: Vec<String>, fields: &mut Vec<String>, collation: Collation) {
    if collation != Collation::Current && (32..=2048).contains(&paths.len()) {
        let bytes = paths.iter().map(|p| p.len() + 1).sum::<usize>();
        if bytes <= 64 * 1024 {
            PREVIOUS.with_borrow_mut(|previous| {
                if !previous.matches(&paths, collation) {
                    previous.update(&mut paths, collation, bytes);
                }
                append_in_order(&mut paths, fields, &previous.indices);
            });
            return;
        }
    }
    #[cfg(target_os = "macos")]
    if collation == Collation::FixedLocale && paths.len() >= 32 {
        let bytes = paths.iter().map(|p| p.len() + 1).sum();
        let mut indices = Vec::new();
        sort_indices(&mut paths, collation, bytes, &mut indices);
        append_in_order(&mut paths, fields, &indices);
        return;
    }
    if collation == Collation::Bytes || paths.len() < 2 {
        paths.sort_unstable();
    } else {
        for path in &mut paths {
            path.push('\0');
        }
        paths.sort_unstable_by(|a, b| unsafe { compare_terminated(a, b) });
        for path in &mut paths {
            path.pop();
        }
    }
    fields.append(&mut paths);
}

fn append_in_order(paths: &mut [String], fields: &mut Vec<String>, indices: &[usize]) {
    fields.extend(indices.iter().map(|&i| std::mem::take(&mut paths[i])));
}
