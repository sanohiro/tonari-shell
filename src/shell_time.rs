use crate::Session;
use std::cell::RefCell;
use std::ffi::CString;

#[cfg(target_env = "musl")]
mod regional;

// Limit padding independently of the literal text supplied by the caller.
// The GNU builtin formats time in a fixed buffer; Tonari also accepts long
// literal formats, but a short prompt cannot request unbounded expansion.
const MAX_TIME_PADDING: usize = 64 * 1024;

fn valid_widths(format: &str) -> bool {
    let mut rest = format;
    let mut total = 0usize;
    while let Some((_, next)) = rest.split_once('%') {
        if let Some(tail) = next.strip_prefix('%') { rest = tail; continue; }
        let width = next.trim_start_matches(['_', '-', '0', '^', '#']);
        let code = width.trim_start_matches(|c: char| c.is_ascii_digit());
        if code.len() != width.len() {
            let Some(n) = width[..width.len() - code.len()].parse::<usize>().ok() else { return false; };
            total = total.saturating_add(n);
            if total > MAX_TIME_PADDING { return false; }
        }
        rest = code;
    }
    true
}

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
        .or_else(|error| {
            if let Some(name) = name.as_deref().filter(|n| n.len() >= 3 && n.bytes().all(|c| c.is_ascii_alphabetic())) {
                tz::TimeZone::from_posix_tz(&format!("{name}0"))
            } else { Err(error) }
        }).unwrap_or_else(|_| tz::TimeZone::utc());
        #[cfg(not(target_env = "musl"))]
        let locale =
            CString::new(locale_name.as_str())
                .ok()
                .map_or(std::ptr::null_mut(), |name| unsafe {
                    libc::newlocale(libc::LC_TIME_MASK, name.as_ptr(), std::ptr::null_mut())
                });
        Self {
            #[cfg(target_env = "musl")]
            region: tonari_time_locales::Locale::find(&locale_name)
                .unwrap_or(tonari_time_locales::POSIX),
            name,
            zone,
            locale_name,
            #[cfg(not(target_env = "musl"))]
            locale,
        }
    }
}

pub(crate) fn format(session: &Session, format: &str, seconds: libc::time_t) -> String {
    try_format(session, format, seconds).unwrap_or_default()
}

pub(crate) fn try_format(session: &Session, format: &str, seconds: i64) -> Option<String> {
    using_clock(session, format, |stamp| stamp(seconds))
}

pub(crate) fn with_formatter<T>(
    session: &Session, format: &str, run: impl FnOnce(&dyn Fn(i64) -> String) -> T,
) -> T {
    using_clock(session, format, |stamp| run(&|seconds| stamp(seconds).unwrap_or_default()))
}

fn using_clock<T>(
    session: &Session,
    format: &str,
    run: impl FnOnce(&dyn Fn(i64) -> Option<String>) -> T,
) -> T {
    if format == "%s" {
        return run(&|seconds| Some(seconds.to_string()));
    }
    if format.as_bytes().iter().any(u8::is_ascii_digit) && !valid_widths(format) {
        return run(&|_| None);
    }
    let name = session
        .is_exported("TZ")
        .then(|| session.get("TZ"))
        .flatten();
    let locale = session.locale_category("LC_TIME").unwrap_or("C".into());
    thread_local! { static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) }; }
    CLOCK.with_borrow_mut(|slot| {
        if slot
            .as_ref()
            .is_none_or(|c| c.name != name || c.locale_name != locale)
        {
            *slot = Some(Clock::new(name, locale.into_owned()));
        }
        let clock = slot.as_ref().unwrap();
        run(&|seconds| format_in_zone(clock, format, seconds))
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
        let remaining = chars.as_str();
        let width = remaining.trim_start_matches(['-', '_', '^', '#', '0']);
        let tail = width.trim_start_matches(|c: char| c.is_ascii_digit());
        let conversion = tail.strip_prefix(['E', 'O']).unwrap_or(tail);
        if conversion.len() != remaining.len()
            && let Some(spec @ ('s' | 'Z' | 'z')) = conversion.chars().next()
        {
            let mut value = match spec {
                's' => seconds.to_string(),
                'Z' => designation.to_owned(),
                _ => {
                    let mut s = String::new();
                    push_zone_offset(&mut s, offset);
                    s
                }
            };
            let flags = &remaining[..remaining.len() - width.len()];
            if spec == 'Z' {
                if flags.contains('#') {
                    value.make_ascii_lowercase();
                } else if flags.contains('^') {
                    value.make_ascii_uppercase();
                }
            }
            let digits = &width[..width.len() - tail.len()];
            let size = if digits.is_empty() {
                0
            } else {
                digits.parse::<i32>().ok()? as usize
            };
            let pad = if flags.ends_with('0') {
                '0'
            } else {
                ' '
            };
            let count = size.saturating_sub(value.len());
            fmt.try_reserve(count).ok()?;
            fmt.extend(std::iter::repeat_n(pad, count));
            fmt.push_str(&value.replace('%', "%%"));
            chars = conversion[1..].chars();
            continue;
        }
        match chars.next() {
            Some('s') => write!(fmt, "{seconds}").ok()?,
            Some('Z') => fmt.push_str(&designation.replace('%', "%%")),
            Some('z') => push_zone_offset(&mut fmt, offset),
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
            bytes.copy_within(1.., 0);
            bytes.pop();
            return Some(String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()));
        }
        if size >= max {
            return None;
        }
        size = size.saturating_mul(2).min(max);
    }
}

fn pair(dst: &mut String, n: i32) {
    dst.push(char::from(b'0' + (n / 10) as u8));
    dst.push(char::from(b'0' + (n % 10) as u8));
}

fn push_zone_offset(out: &mut String, offset: i32) {
    let minutes = (offset / 60).unsigned_abs();
    out.push(if offset < 0 { '-' } else { '+' });
    let hours = minutes / 60;
    if hours < 100 {
        pair(out, hours as i32);
    } else {
        use std::fmt::Write;
        write!(out, "{hours}").unwrap();
    }
    pair(out, (minutes % 60) as i32);
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
            b'z' => push_zone_offset(&mut result, offset),
            b'Z' => result.push_str(zone),
            b'%' => result.push('%'),
            _ => return None,
        }
        rest = &after[1..];
    }
    result.push_str(rest);
    Some(result)
}
