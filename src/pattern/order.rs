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

pub(super) fn invalidate() {
    PREVIOUS.with_borrow_mut(|order| order.indices.clear());
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

// A fixed-width prefix keeps comparisons in the sort array. File names
// cannot contain NUL, so zero padding sorts a shorter name first. Compare
// the complete strings only within groups that share a prefix. Large inputs leave four bytes for the index.
fn sort_bytes(paths: &[String], indices: &mut [usize]) {
    if paths.len() < 256 || paths.len() > u32::MAX as usize {
        indices.sort_unstable_by(|&i, &j| paths[i].cmp(&paths[j]));
        return;
    }
    // The known directory prefix is usually identical across this expansion.
    let first = paths[0].as_bytes();
    let mut common = first.len();
    for path in &paths[1..] {
        let bytes = path.as_bytes();
        common = common.min(bytes.len());
        if first[..common] != bytes[..common] {
            common = first[..common]
                .iter()
                .zip(bytes)
                .take_while(|(a, b)| a == b)
                .count();
        }
        if common == 0 {
            break;
        }
    }
    let index_bits = if paths.len() <= usize::from(u16::MAX) {
        16
    } else {
        32
    };
    let prefix_len = 8 - index_bits / 8;
    let index_mask = (1u64 << index_bits) - 1;
    let mut keys = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        let mut key = [0; 8];
        let bytes = &path.as_bytes()[common..];
        let take = bytes.len().min(prefix_len);
        key[..take].copy_from_slice(&bytes[..take]);
        keys.push(u64::from_be_bytes(key) | index as u64);
    }
    keys.sort_unstable();
    let mut tail = indices;
    for group in keys.chunk_by(|a, b| a >> index_bits == b >> index_bits) {
        let (range, rest) = tail.split_at_mut(group.len());
        for (i, key) in range.iter_mut().zip(group) {
            *i = (*key & index_mask) as usize;
        }
        if range.len() > 1 {
            range.sort_unstable_by(|&i, &j| paths[i].cmp(&paths[j]));
        }
        tail = rest;
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
        sort_bytes(paths, indices);
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
fn sort_large(
    indices: &mut [usize],
    compare: &(impl Fn(&usize, &usize) -> Ordering + Sync),
    workers: usize,
) {
    if workers <= 1 || indices.len() < 4096 {
        indices.sort_unstable_by(compare);
        return;
    }
    let (left, right) = indices.split_at_mut(indices.len() / 2);
    // The enclosing glob keeps this locale alive until the worker joins.
    let locale = unsafe { libc::uselocale(std::ptr::null_mut()) } as usize;
    let joined = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new().spawn_scoped(scope, || {
            // POSIX thread locales do not propagate to newly created threads.
            let _locale = super::LocaleGuard::enter(locale as libc::locale_t);
            sort_large(left, compare, workers / 2);
        });
        sort_large(right, compare, workers / 2);
        if let Ok(handle) = worker {
            handle.join().unwrap();
            true
        } else {
            false
        }
    });
    if !joined {
        sort_large(left, compare, workers / 2);
    }
    let mut result = Vec::with_capacity(left.len() + right.len());
    let (mut a, mut b) = (&left[..], &right[..]);
    while !a.is_empty() && !b.is_empty() {
        if compare(&a[0], &b[0]) != Ordering::Greater {
            result.push(a[0]);
            a = &a[1..];
        } else {
            result.push(b[0]);
            b = &b[1..];
        }
    }
    result.extend_from_slice(a);
    result.extend_from_slice(b);
    indices.copy_from_slice(&result);
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
    for path in paths.iter() {
        offsets.push(wide.len());
        wide.extend(path.chars().map(|c| c as libc::wchar_t));
        wide.push(0);
    }
    let compare = |&i: &usize, &j: &usize| {
        let order = unsafe { wcscoll(wide[offsets[i]..].as_ptr(), wide[offsets[j]..].as_ptr()) };
        order.cmp(&0).then_with(|| paths[i].cmp(&paths[j]))
    };
    // Locale order often has long sorted runs in byte order (e.g. numbered
    // paths). The adaptive sort compares with the real locale and merges runs.
    if indices.len() < 256 {
        indices.sort_unstable_by(compare);
        return true;
    }
    // A small sample distinguishes long ordered runs from unrelated byte and
    // collation orders. It chooses an algorithm only; both use the full keys.
    if indices.len() >= 4096 {
        let stride = indices.len() / 128;
        let mut sample: [usize; 128] = std::array::from_fn(|i| i * stride);
        sample.sort_unstable_by(|&i, &j| paths[i].cmp(&paths[j]));
        if sample
            .windows(2)
            .filter(|pair| compare(&pair[0], &pair[1]).is_gt())
            .count()
            > 16
        {
            let limit = if indices.len() >= 32768 { 4 } else { 2 };
            let workers = std::thread::available_parallelism().map_or(1, |p| p.get().min(limit));
            sort_large(indices, &compare, workers);
            return true;
        }
    }
    sort_bytes(paths, indices);
    indices.sort_by(compare);
    true
}

// Safety: both paths must have a trailing NUL for the duration of the call.
unsafe fn compare_terminated(a: &str, b: &str) -> Ordering {
    let order = unsafe { libc::strcoll(a.as_ptr().cast(), b.as_ptr().cast()) };
    order.cmp(&0).then_with(|| a.cmp(b))
}

pub(super) fn sort_mode(value: Option<&str>) -> (&str, bool) {
    let mode = value.unwrap_or("").trim_start_matches([' ', '\t']);
    if let Some(key) = mode.strip_prefix('-') {
        (key, true)
    } else {
        (mode.strip_prefix('+').unwrap_or(mode), false)
    }
}

pub(super) fn append_expansion(
    paths: Vec<String>,
    fields: &mut Vec<String>,
    collation: Collation,
    sorting: Option<&str>,
) {
    use std::os::unix::fs::MetadataExt;
    let (key, reverse) = sort_mode(sorting);
    if key == "nosort" {
        fields.extend(paths);
        return;
    }
    let start = fields.len();
    append(paths, fields, collation);
    let paths = &mut fields[start..];
    match key {
        "" | "name" => {}
        "numeric" => paths.sort_by_cached_key(|p| {
            if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
                (false, p.parse::<i64>().unwrap_or(i64::MAX))
            } else {
                (true, 0)
            }
        }),
        "size" | "blocks" | "mtime" | "ctime" | "atime" => paths.sort_by_cached_key(|p| {
            std::fs::metadata(p).map_or((-1, -1), |m| match key {
                "size" => (m.size() as i64, 0),
                "blocks" => (m.blocks() as i64, 0),
                "mtime" => (m.mtime(), m.mtime_nsec()),
                "ctime" => (m.ctime(), m.ctime_nsec()),
                _ => (m.atime(), m.atime_nsec()),
            })
        }),
        _ => return,
    }
    if reverse {
        paths.reverse();
    }
}

pub(super) fn append(mut paths: Vec<String>, fields: &mut Vec<String>, collation: Collation) {
    if collation != Collation::Current && (32..=16384).contains(&paths.len()) {
        let bytes = paths.iter().map(|p| p.len() + 1).sum::<usize>();
        if bytes <= 512 * 1024 {
            PREVIOUS.with_borrow_mut(|previous| {
                if !previous.matches(&paths, collation) {
                    previous.update(&mut paths, collation, bytes);
                }
                append_in_order(&mut paths, fields, &previous.indices);
            });
            return;
        }
    }
    if (collation == Collation::Bytes && paths.len() > 16384)
        || (cfg!(target_os = "macos") && collation == Collation::FixedLocale && paths.len() >= 32)
    {
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

// Completion lists use byte order and do not retain names between Tab presses.
pub(crate) fn sorted_names(mut names: Vec<String>) -> Vec<String> {
    if names.len() < 1024 {
        names.sort_unstable();
        return names;
    }
    let mut indices: Vec<_> = (0..names.len()).collect();
    sort_bytes(&names, &mut indices);
    let mut sorted = Vec::with_capacity(names.len());
    append_in_order(&mut names, &mut sorted, &indices);
    sorted
}

fn append_in_order(paths: &mut [String], fields: &mut Vec<String>, indices: &[usize]) {
    fields.extend(indices.iter().map(|&i| std::mem::take(&mut paths[i])));
}
