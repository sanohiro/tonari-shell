use super::order::{self, Collation};
use std::cell::RefCell;
use std::ffi::CString;

struct Cached {
    ctype: String,
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
    work: impl FnOnce(Collation) -> T,
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
                order::invalidate();
            }
        }
        let Some(locale) = slot else {
            return work(Collation::Current);
        };
        let previous = unsafe { libc::uselocale(locale.handle) };
        if previous.is_null() {
            return work(Collation::Current);
        }
        let order = if matches!(locale.collate.as_str(), "C" | "POSIX") {
            Collation::Bytes
        } else {
            Collation::FixedLocale
        };
        struct Restore(libc::locale_t);
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    libc::uselocale(self.0);
                }
            }
        }
        let _restore = Restore(previous);
        work(order)
    })
}
