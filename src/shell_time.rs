use crate::Session;
use std::cell::RefCell;
use std::ffi::CString;

struct Clock {
    name: Option<String>,
    zone: tz::TimeZone,
    locale_name: String,
    locale: libc::locale_t,
}

impl Drop for Clock {
    fn drop(&mut self) {
        if !self.locale.is_null() {
            unsafe { libc::freelocale(self.locale) };
        }
    }
}

impl Clock {
    fn new(name: Option<String>, locale_name: String) -> Self {
        let zone = match name.as_deref() {
            None => tz::TimeZone::local(),
            Some("") => Ok(tz::TimeZone::utc()),
            Some(name) => tz::TimeZone::from_posix_tz(name),
        }
        .unwrap_or_else(|_| tz::TimeZone::utc());
        let locale =
            CString::new(locale_name.as_str())
                .ok()
                .map_or(std::ptr::null_mut(), |name| unsafe {
                    libc::newlocale(libc::LC_TIME_MASK, name.as_ptr(), std::ptr::null_mut())
                });
        Self {
            name,
            zone,
            locale_name,
            locale,
        }
    }
}

pub(crate) fn format(session: &Session, format: &str, seconds: libc::time_t) -> String {
    if format == "%s" {
        return seconds.to_string();
    }
    let name = session
        .is_exported("TZ")
        .then(|| session.get("TZ"))
        .flatten();
    let locale = ["LC_ALL", "LC_TIME", "LANG"]
        .into_iter()
        .find_map(|name| session.get(name).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "C".into());
    thread_local! { static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) }; }
    CLOCK.with_borrow_mut(|slot| {
        if slot
            .as_ref()
            .is_none_or(|c| c.name != name || c.locale_name != locale)
        {
            *slot = Some(Clock::new(name, locale));
        }
        let clock = slot.as_ref().unwrap();
        format_in_zone(clock, format, seconds).unwrap_or_default()
    })
}

fn format_in_zone(clock: &Clock, format: &str, seconds: i64) -> Option<String> {
    let local = clock.zone.find_local_time_type(seconds).ok()?;
    let offset = local.ut_offset();
    let shifted = seconds.checked_add(i64::from(offset))?;
    let mut tm = std::mem::MaybeUninit::uninit();
    if unsafe { libc::gmtime_r(&shifted, tm.as_mut_ptr()) }.is_null() {
        return None;
    }
    let mut tm = unsafe { tm.assume_init() };
    tm.tm_isdst = i32::from(local.is_dst());
    tm.tm_gmtoff = offset.into();

    let designation = local.time_zone_designation();
    let designation = if designation.is_empty() {
        "UTC"
    } else {
        designation
    };
    let zone_name = CString::new(designation).ok()?;
    tm.tm_zone = zone_name.as_ptr().cast_mut();

    // strftime's %s can consult the process zone; musl also verifies its own
    // zone pointers for %Z. Render those directives from the shell's zone.
    use std::fmt::Write;
    let mut fmt = String::with_capacity(format.len() + 2);
    fmt.push(' ');
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            fmt.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => write!(fmt, "{seconds}").ok()?,
            Some('Z') => fmt.push_str(&designation.replace('%', "%%")),
            Some('z') => {
                let minutes = offset.unsigned_abs() / 60;
                write!(
                    fmt,
                    "{}{:02}{:02}",
                    if offset < 0 { '-' } else { '+' },
                    minutes / 60,
                    minutes % 60
                )
                .ok()?;
            }
            Some(c) => {
                fmt.push('%');
                fmt.push(c);
            }
            None => fmt.push('%'),
        }
    }
    // A literal prefix distinguishes an empty conversion from a short buffer.
    let mut size = (fmt.len() + 1).max(128);
    let fmt = std::ffi::CString::new(fmt).ok()?;
    // Some libc implementations also return zero on an invalid directive.
    // Bound retries by the input, so a rejected format cannot
    // keep doubling the buffer. Arbitrarily long ordinary formats still fit.
    let max = fmt.as_bytes().len().saturating_mul(256).max(size);
    loop {
        let mut bytes = Vec::<u8>::new();
        bytes.try_reserve_exact(size).ok()?;
        let n = unsafe {
            if clock.locale.is_null() {
                libc::strftime(bytes.as_mut_ptr().cast(), size, fmt.as_ptr(), &tm)
            } else {
                libc::strftime_l(
                    bytes.as_mut_ptr().cast(),
                    size,
                    fmt.as_ptr(),
                    &tm,
                    clock.locale,
                )
            }
        };
        if n > 0 {
            unsafe { bytes.set_len(n) };
            return Some(String::from_utf8_lossy(&bytes[1..]).into_owned());
        }
        if size >= max {
            return None;
        }
        size = size.saturating_mul(2).min(max);
    }
}
