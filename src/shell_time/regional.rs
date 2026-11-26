// musl ships only C-locale time strings. Take the localized parts from the
// GNU locale data; the clock and calendar conversions still use the shell TZ.
use tonari_time_locales::{Field, Locale};

pub(super) fn resolve(name: &str) -> Locale {
    let normalized;
    let name = if let Some((base, codeset)) = name.split_once('.') {
        if let Some((_, modifier)) = codeset.split_once('@') {
            normalized = format!("{base}@{modifier}");
            &normalized
        } else {
            base
        }
    } else {
        name
    };
    if matches!(name, "C" | "POSIX") {
        return tonari_time_locales::POSIX;
    }
    Locale::find(name).unwrap_or(tonari_time_locales::POSIX)
}

pub(super) fn expand(
    locale: Locale,
    format: &str,
    tm: &libc::tm,
    single: &dyn Fn(&str) -> Option<String>,
) -> String {
    let mut result = String::with_capacity(format.len() + 24);
    expand_into(&mut result, locale, format, tm, 0, single);
    result
}

fn literal(out: &mut String, text: &str) {
    if !text.contains('%') {
        out.push_str(text);
        return;
    }
    for ch in text.chars() {
        out.push(ch);
        if ch == '%' {
            out.push('%');
        }
    }
}

fn expand_into(
    out: &mut String,
    locale: Locale,
    format: &str,
    tm: &libc::tm,
    depth: u8,
    single: &dyn Fn(&str) -> Option<String>,
) {
    let mut rest = format;
    while let Some((prefix, after)) = rest.split_once('%') {
        out.push_str(prefix);
        let flags = after.trim_start_matches(['_', '-', '0', '^', '#']);
        let code = flags.trim_start_matches(|ch: char| ch.is_ascii_digit());
        let flags_len = after.len() - flags.len();
        let prefix_len = after.len() - code.len();
        let mut chars = code.chars();
        let first = chars.next();
        let modifier = first.filter(|c| matches!(c, 'E' | 'O'));
        let spec = if modifier.is_some() {
            chars.next()
        } else {
            first
        };
        rest = chars.as_str();
        let directive = &after[..after.len() - rest.len()];
        let valid = match (modifier, spec) {
            (Some('E'), Some(c)) => "cCxXyY".contains(c),
            (Some('O'), Some(c)) => "bBhdeHIklmMSuUVwWy".contains(c),
            (None, Some(c)) => "aAbBhCcDdeFgGHIjklmMnPprRsStTuUVwWxXyYzZ%".contains(c),
            _ => false,
        };
        if !valid {
            // GNU strftime preserves the spelling of an unknown conversion.
            out.push_str("%%");
            literal(out, directive);
            continue;
        }
        if prefix_len > 0 {
            if prefix_len == 1
                && modifier.is_none()
                && short_number(out, tm, after.as_bytes()[0], spec.unwrap())
            {
                continue;
            }
            let mut plain = String::from("%");
            plain.push_str(&directive[prefix_len..]);
            if let Some(value) = single(&plain) {
                decorated(
                    out,
                    &value,
                    spec.unwrap(),
                    &after[..flags_len],
                    &after[flags_len..prefix_len],
                );
            }
            continue;
        }
        // GNU locale templates use %k and %l. musl rejects either directive
        // and otherwise discards the entire formatted timestamp.
        if modifier.is_none() && matches!(spec, Some('k' | 'l')) {
            short_number(out, tm, b'_', spec.unwrap());
            continue;
        }
        let day = tm.tm_wday as usize;
        let month = tm.tm_mon as usize;
        let name = match (modifier, spec) {
            (None, Some('a')) => locale.get(Field::Abday, day),
            (None, Some('A')) => locale.get(Field::Day, day),
            (_, Some('b' | 'h')) if modifier != Some('E') => {
                let base = locale.get(Field::Abmon, month);
                if modifier == Some('O') {
                    locale.get(Field::AbAltMon, month).or(base)
                } else {
                    base
                }
            }
            (_, Some('B')) if modifier != Some('E') => {
                let base = locale.get(Field::Mon, month);
                if modifier == Some('O') {
                    locale.get(Field::AltMon, month).or(base)
                } else {
                    base
                }
            }
            (None, Some('p' | 'P')) => locale.get(Field::AmPm, usize::from(tm.tm_hour >= 12)),
            (Some('O'), Some(c)) if "deHIklmMSuUVwWy".contains(c) => {
                numeric_value(tm, c).and_then(|value| locale.get(Field::AltDigits, value))
            }
            _ => None,
        };
        if let Some(text) = name {
            if spec == Some('P') {
                literal(out, &text.to_lowercase());
            } else {
                literal(out, text);
            }
            continue;
        }
        let nested = match (modifier, spec) {
            (None | Some('E'), Some('c')) => Some(
                if modifier.is_some() {
                    locale.get(Field::EraDTFmt, 0).filter(|s| !s.is_empty())
                } else {
                    None
                }
                .unwrap_or_else(|| locale.get(Field::DTFmt, 0).unwrap_or("")),
            ),
            (None | Some('E'), Some('x')) => Some(
                if modifier.is_some() {
                    locale.get(Field::EraDFmt, 0).filter(|s| !s.is_empty())
                } else {
                    None
                }
                .unwrap_or_else(|| locale.get(Field::DFmt, 0).unwrap_or("")),
            ),
            (None | Some('E'), Some('X')) => Some(
                if modifier.is_some() {
                    locale.get(Field::EraTFmt, 0).filter(|s| !s.is_empty())
                } else {
                    None
                }
                .unwrap_or_else(|| locale.get(Field::TFmt, 0).unwrap_or("")),
            ),
            (None, Some('r')) => {
                let s = locale.get(Field::TFmtAmpm, 0).unwrap_or("");
                Some(if s.is_empty() { "%I:%M:%S %p" } else { s })
            }
            (Some('E'), Some('C' | 'Y' | 'y')) => {
                if let Some((name, year, fmt)) = era(locale, tm) {
                    match spec {
                        Some('C') => {
                            literal(out, name);
                            continue;
                        }
                        Some('y') => {
                            use std::fmt::Write;
                            let _ = write!(out, "{year:02}");
                            continue;
                        }
                        _ => Some(fmt),
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(nested) = nested.filter(|_| depth < 8) {
            expand_into(out, locale, nested, tm, depth + 1, single);
            continue;
        }
        out.push('%');
        // The ordinary numeric conversion is the fallback if this locale has
        // no era or alternative digit for the requested value.
        let ordinary = match (modifier, spec) {
            (Some('E'), Some(c)) => "CYy".contains(c),
            (Some('O'), Some(c)) => "deHIklmMSuUVwWy".contains(c),
            _ => false,
        };
        if let Some(m) = modifier.filter(|_| !ordinary) {
            out.push(m);
        }
        if let Some(c) = spec {
            out.push(c);
        }
    }
    out.push_str(rest);
}

// Common padding flags operate on an already-converted calendar field.
fn short_number(out: &mut String, tm: &libc::tm, flag: u8, spec: char) -> bool {
    if !matches!(flag, b'0' | b'_' | b'-') {
        return false;
    }
    let number = match spec {
        'd' | 'e' => tm.tm_mday,
        'H' | 'k' => tm.tm_hour,
        'I' | 'l' => (tm.tm_hour + 11) % 12 + 1,
        'm' => tm.tm_mon + 1,
        'M' => tm.tm_min,
        'S' => tm.tm_sec,
        'y' => tm.tm_year.rem_euclid(100),
        _ => return false,
    } as u8;
    if number >= 10 {
        out.push(char::from(b'0' + number / 10));
    } else if flag != b'-' {
        out.push(if flag == b'0' { '0' } else { ' ' });
    }
    out.push(char::from(b'0' + number % 10));
    true
}

// Text widths in strftime count bytes, including multibyte month names.
fn decorated(out: &mut String, value: &str, spec: char, flags: &str, width: &str) {
    let upper = flags.contains('^') || flags.contains('#') && "aAbBh".contains(spec);
    let lower = flags.contains('#') && "pZ".contains(spec);
    let changed;
    let value = if lower {
        changed = value.to_lowercase();
        changed.as_str()
    } else if upper {
        changed = value.to_uppercase();
        changed.as_str()
    } else {
        value
    };
    let padding = flags
        .bytes()
        .rev()
        .find(|c| matches!(c, b'-' | b'_' | b'0'));
    let numeric =
        "CdeGgHIjklmMSuUVwWyY".contains(spec) && value.trim().bytes().all(|b| b.is_ascii_digit());
    let default = if numeric && !matches!(spec, 'e' | 'k' | 'l') {
        '0'
    } else {
        ' '
    };
    let pad = match padding {
        Some(b'0') => '0',
        Some(b'-' | b'_') => ' ',
        _ => default,
    };
    let natural = if numeric && padding != Some(b'-') {
        value.len()
    } else {
        0
    };
    let value = if numeric && (padding.is_some() || !width.is_empty()) {
        let digits = value.trim_start_matches(['0', ' ']);
        if digits.is_empty() { "0" } else { digits }
    } else {
        value
    };
    let width = if width.is_empty() {
        0
    } else {
        let Ok(size) = width.parse::<i32>() else {
            return;
        };
        size as usize
    };
    let count = width.max(natural).saturating_sub(value.len());
    if out.try_reserve(count).is_ok() {
        out.extend(std::iter::repeat_n(pad, count));
        literal(out, value);
    }
}

fn numeric_value(tm: &libc::tm, spec: char) -> Option<usize> {
    let fmt = [b'%', spec as u8, 0];
    let mut buf = [0u8; 8];
    let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr().cast(), tm) };
    std::str::from_utf8(&buf[..n]).ok()?.trim().parse().ok()
}

fn date(s: &str) -> Option<(i32, i32, i32)> {
    let mut fields = s.split('/');
    let year: i32 = fields.next()?.parse().ok()?;
    // Era data uses 1 BC, whereas struct tm uses astronomical year 0.
    Some((
        if year < 0 { year + 1 } else { year },
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
    ))
}

fn era(locale: Locale, tm: &libc::tm) -> Option<(&'static str, i64, &'static str)> {
    let current = (tm.tm_year.checked_add(1900)?, tm.tm_mon + 1, tm.tm_mday);
    for entry in locale.values(Field::Era) {
        let mut fields = entry.splitn(6, ':');
        let direction = fields.next()?;
        let base: i64 = fields.next()?.parse().ok()?;
        let start = date(fields.next()?)?;
        let end = match fields.next()? {
            "+*" => (i32::MAX, 12, 31),
            "-*" => (i32::MIN, 1, 1),
            s => date(s)?,
        };
        if (start.min(end)..=start.max(end)).contains(&current) {
            let delta = i64::from(current.0) - i64::from(start.0);
            let year = if (start <= end) == (direction == "+") {
                base + delta
            } else {
                base - delta
            };
            return Some((fields.next()?, year, fields.next()?));
        }
    }
    None
}
