use super::CharacterLocale;
use super::order::{self, Collation};
use std::cell::RefCell;
use std::ffi::CString;

struct Cached {
    ctype: String,
    characters: CharacterLocale,
    collate: String,
    handle: libc::locale_t,
}

impl Drop for Cached {
    fn drop(&mut self) {
        unsafe { libc::freelocale(self.handle) };
    }
}

impl Cached {
    fn new(ctype: String, collate: String) -> Option<Self> {
        let characters = CString::new(ctype.as_str()).ok()?;
        let ordering = CString::new(collate.as_str()).ok()?;
        let base = unsafe {
            libc::newlocale(
                libc::LC_CTYPE_MASK,
                characters.as_ptr(),
                std::ptr::null_mut(),
            )
        };
        if base.is_null() {
            return None;
        }
        let handle = unsafe { libc::newlocale(libc::LC_COLLATE_MASK, ordering.as_ptr(), base) };
        if handle.is_null() {
            unsafe { libc::freelocale(base) };
            return None;
        }
        Some(Self {
            characters: CharacterLocale::from_name(&ctype),
            ctype,
            collate,
            handle,
        })
    }
}

// The shell can change its locale variables without exporting them to libc.
// Construct a locale only on the first actual pathname pattern for a setting.
pub(super) fn run<T>(
    lookup: &dyn Fn(&str) -> Option<String>,
    work: impl FnOnce(Collation, CharacterLocale) -> T,
) -> T {
    thread_local! { static LAST: RefCell<Option<Cached>> = const { RefCell::new(None) }; }
    let nonempty = |key| lookup(key).filter(|s| !s.is_empty());
    let (ctype, collate) = if let Some(all) = nonempty("LC_ALL") {
        (all.clone(), all)
    } else {
        let lang = nonempty("LANG").unwrap_or_else(|| "C".into());
        (
            nonempty("LC_CTYPE").unwrap_or_else(|| lang.clone()),
            nonempty("LC_COLLATE").unwrap_or(lang),
        )
    };
    LAST.with_borrow_mut(|slot| {
        if !slot
            .as_ref()
            .is_some_and(|c| c.ctype == ctype && c.collate == collate)
        {
            if let Some(next) = Cached::new(ctype, collate) {
                *slot = Some(next);
                if !cfg!(target_os = "linux") {
                    order::invalidate();
                }
            }
        }
        let Some(locale) = slot else {
            return work(Collation::Current, CharacterLocale::Ascii);
        };
        let Some(_guard) = super::LocaleGuard::enter(locale.handle) else {
            return work(Collation::Current, locale.characters);
        };
        let order = if matches!(locale.collate.as_str(), "C" | "POSIX") {
            Collation::Bytes
        } else {
            Collation::FixedLocale
        };
        work(order, locale.characters)
    })
}

// A script may change LC_COLLATE or the shopt between two matches against
// the same compiled case pattern. Keep the comparison mode out of its AST.
thread_local! {
    static RANGE_CACHE: RefCell<Option<Cached>> = const { RefCell::new(None) };
    static RANGE_HANDLE: std::cell::Cell<libc::locale_t> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

pub(crate) fn sync_ranges(session: &crate::Session) {
    if session.shopt.globasciiranges {
        RANGE_HANDLE.set(std::ptr::null_mut());
        return;
    }
    let name = ["LC_ALL", "LC_COLLATE", "LANG"]
        .into_iter()
        .find_map(|key| session.scalar_setting(key).filter(|s| !s.is_empty()))
        .unwrap_or_default();
    if matches!(name.as_ref(), "" | "C" | "POSIX" | "C.UTF-8" | "C.utf8") {
        RANGE_HANDLE.set(std::ptr::null_mut());
        return;
    }
    RANGE_CACHE.with_borrow_mut(|cached| {
        if cached.as_ref().is_none_or(|x| x.collate != name) {
            RANGE_HANDLE.set(std::ptr::null_mut());
            let owned = name.into_owned();
            *cached = Cached::new(owned.clone(), owned);
        }
        RANGE_HANDLE.set(cached.as_ref().map_or(std::ptr::null_mut(), |x| x.handle));
    });
}

pub(super) fn collated_ranges() -> bool {
    !RANGE_HANDLE.get().is_null()
}

pub(super) fn range_order(a: &[u8; 4], b: &[u8; 4]) -> std::cmp::Ordering {
    compare(RANGE_HANDLE.get(), a, b)
}

pub(super) fn in_range(lo: &[u8; 4], value: &[u8; 4], hi: &[u8; 4]) -> bool {
    let locale = RANGE_HANDLE.get();
    if locale.is_null() {
        return lo <= value && value <= hi;
    }
    !compare(locale, lo, value).is_gt() && !compare(locale, value, hi).is_gt()
}

fn compare(locale: libc::locale_t, a: &[u8; 4], b: &[u8; 4]) -> std::cmp::Ordering {
    if locale.is_null() || a == b {
        return a.cmp(b);
    }
    unsafe extern "C" {
        fn strcoll_l(
            a: *const libc::c_char,
            b: *const libc::c_char,
            locale: libc::locale_t,
        ) -> libc::c_int;
    }
    let mut left = [0u8; 5];
    let mut right = [0u8; 5];
    left[..4].copy_from_slice(a);
    right[..4].copy_from_slice(b);
    unsafe { strcoll_l(left.as_ptr().cast(), right.as_ptr().cast(), locale) }.cmp(&0)
}
