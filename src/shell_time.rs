use crate::Session;
use std::cell::RefCell;
use std::ffi::CString;

#[cfg(target_env = "musl")]
mod regional;

struct Clock {
    #[cfg(target_env = "musl")]
    region: tonari_time_locales::Locale,
    name: Option<String>,
    zone: tz::TimeZone,
    locale_name: String,
    #[cfg(not(target_env = "musl"))]
    locale: libc::locale_t,
}

#[cfg(not(target_env = "musl"))]
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
            Some(name) => tz::TimeZone::from_posix_tz(name).or_else(|error| {
                // Recent slim zoneinfo installations omit some historical US
                // names. Prefer their IANA region, including historical rules.
                let region = match name {
                    "EST5EDT" => "America/New_York",
                    "CST6CDT" => "America/Chicago",
                    "MST7MDT" => "America/Denver",
                    "PST8PDT" => "America/Los_Angeles",
                    _ => return Err(error),
                };
                tz::TimeZone::from_posix_tz(region)
                    .or_else(|_| tz::TimeZone::from_posix_tz(&format!("{name},M3.2.0/2,M11.1.0/2")))
            }),
        }
        .unwrap_or_else(|_| tz::TimeZone::utc());
        #[cfg(not(target_env = "musl"))]
        let locale =
            CString::new(locale_name.as_str())
                .ok()
                .map_or(std::ptr::null_mut(), |name| unsafe {
                    libc::newlocale(libc::LC_TIME_MASK, name.as_ptr(), std::ptr::null_mut())
                });
        Self {
            #[cfg(target_env = "musl")]
            region: regional::resolve(&locale_name),
            name,
            zone,
            locale_name,
            #[cfg(not(target_env = "musl"))]
            locale,
        }
    }
}

pub(crate) fn format(session: &Session, format: &str, seconds: libc::time_t) -> String {
    with_formatter(session, format, |stamp| stamp(seconds))
}

pub(crate) fn with_formatter<T>(
    session: &Session,
    format: &str,
    run: impl FnOnce(&dyn Fn(i64) -> String) -> T,
) -> T {
    if format == "%s" {
        return run(&|seconds| seconds.to_string());
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
        run(&|seconds| format_in_zone(clock, format, seconds).unwrap_or_default())
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
    if let Some(text) = numeric_time(format, seconds, &tm, designation, offset) {
        return Some(text);
    }
    #[cfg(target_env = "musl")]
    let localized;
    #[cfg(target_env = "musl")]
    let format = {
        localized = regional::expand(clock.region, format, &tm, &|spec| {
            format_in_zone(clock, spec, seconds)
        });
        if !localized.contains('%') {
            return Some(localized);
        }
        if let Some(text) = numeric_time(&localized, seconds, &tm, designation, offset) {
            return Some(text);
        }
        localized.as_str()
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
            #[cfg(target_env = "musl")]
            {
                libc::strftime(bytes.as_mut_ptr().cast(), size, fmt.as_ptr(), &tm)
            }
            #[cfg(not(target_env = "musl"))]
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

// Numeric log timestamps have no locale text or alternate digits. Render them
// directly without passing each numeric component through libc's formatter.
fn numeric_time(
    format: &str,
    seconds: i64,
    tm: &libc::tm,
    zone: &str,
    offset: i32,
) -> Option<String> {
    use std::fmt::Write;
    let year = tm.tm_year.checked_add(1900)?;
    if !(1000..=9999).contains(&year) {
        return None;
    }
    fn pair(dst: &mut String, n: i32) {
        dst.push(char::from(b'0' + (n / 10) as u8));
        dst.push(char::from(b'0' + (n % 10) as u8));
    }
    if let Some((_, after)) = format.split_once('%')
        && !matches!(
            after.as_bytes().first(),
            Some(
                b'Y' | b'F'
                    | b'm'
                    | b'd'
                    | b'H'
                    | b'M'
                    | b'S'
                    | b'T'
                    | b'R'
                    | b's'
                    | b'z'
                    | b'Z'
                    | b'%'
            )
        )
    {
        return None;
    }
    let mut result = String::with_capacity(format.len() + 16);
    let mut rest = format;
    while let Some((text, after)) = rest.split_once('%') {
        result.push_str(text);
        let (&byte, _) = after.as_bytes().split_first()?;
        match byte {
            b'Y' | b'F' => {
                pair(&mut result, year / 100);
                pair(&mut result, year % 100);
                if byte == b'F' {
                    result.push('-');
                    pair(&mut result, tm.tm_mon + 1);
                    result.push('-');
                    pair(&mut result, tm.tm_mday);
                }
            }
            b'm' => pair(&mut result, tm.tm_mon + 1),
            b'd' => pair(&mut result, tm.tm_mday),
            b'H' => pair(&mut result, tm.tm_hour),
            b'M' => pair(&mut result, tm.tm_min),
            b'S' => pair(&mut result, tm.tm_sec),
            b'T' | b'R' => {
                pair(&mut result, tm.tm_hour);
                result.push(':');
                pair(&mut result, tm.tm_min);
                if byte == b'T' {
                    result.push(':');
                    pair(&mut result, tm.tm_sec);
                }
            }
            b's' => write!(result, "{seconds}").ok()?,
            b'z' => {
                let minutes = (offset / 60).abs();
                if minutes >= 6000 {
                    return None;
                }
                result.push(if offset < 0 { '-' } else { '+' });
                pair(&mut result, minutes / 60);
                pair(&mut result, minutes % 60);
            }
            b'Z' => result.push_str(zone),
            b'%' => result.push('%'),
            _ => return None,
        }
        rest = &after[1..];
    }
    result.push_str(rest);
    Some(result)
}
