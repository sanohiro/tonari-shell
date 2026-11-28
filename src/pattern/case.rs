use super::*;

pub(super) fn mapped_char(c: char, upper: bool, locale: CharacterLocale) -> char {
    if c.is_ascii() && !matches!(locale, CharacterLocale::Turkic) {
        return if upper {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        };
    }
    mapped_in(c, upper, locale, utf8_locale())
}

fn utf8_locale() -> libc::locale_t {
    // The shell may start in C and switch to UTF-8. Select a UTF-8 ctype
    // explicitly without changing the process locale or loading Unicode tables.
    static UTF8: OnceLock<usize> = OnceLock::new();
    *UTF8.get_or_init(|| {
        for name in [c"C.UTF-8", c"en_US.UTF-8", c"UTF-8"] {
            let locale = unsafe {
                libc::newlocale(libc::LC_CTYPE_MASK, name.as_ptr(), std::ptr::null_mut())
            };
            if !locale.is_null() {
                return locale as usize;
            }
        }
        0
    }) as libc::locale_t
}

fn mapped_in(c: char, upper: bool, locale: CharacterLocale, utf8: libc::locale_t) -> char {
    unsafe extern "C" {
        fn towupper_l(c: u32, locale: libc::locale_t) -> u32;
        fn towlower_l(c: u32, locale: libc::locale_t) -> u32;
    }
    match (locale, upper, c) {
        (CharacterLocale::Ascii, true, _) => c.to_ascii_uppercase(),
        (CharacterLocale::Ascii, false, _) => c.to_ascii_lowercase(),
        (CharacterLocale::Turkic, true, 'i') => 'İ',
        (CharacterLocale::Turkic, false, 'I') => 'ı',
        // Bash uses the single-code-point mapping. musl's uppercase
        // sharp S differs from both of the reference distributions.
        (_, true, 'ß') => c,
        (_, false, 'İ') => 'i',
        _ if utf8.is_null() => c,
        _ => char::from_u32(unsafe {
            if upper {
                towupper_l(c as u32, utf8)
            } else {
                towlower_l(c as u32, utf8)
            }
        })
        .unwrap_or(c),
    }
}

pub(crate) fn same_letter(left: char, right: char, locale: CharacterLocale) -> bool {
    if left == right {
        return true;
    }
    if left.is_ascii() && right.is_ascii() && !matches!(locale, CharacterLocale::Turkic) {
        return left.eq_ignore_ascii_case(&right);
    }
    mapped_char(left, false, locale) == mapped_char(right, false, locale)
}

pub(crate) fn folded_prefix(value: &str, prefix: &str, locale: CharacterLocale) -> Option<usize> {
    if !matches!(locale, CharacterLocale::Turkic)
        && prefix.is_ascii()
        && let Some(head) = value.as_bytes().get(..prefix.len())
        && head.is_ascii()
    {
        return head
            .eq_ignore_ascii_case(prefix.as_bytes())
            .then_some(prefix.len());
    }
    let mut chars = value.chars();
    for expected in prefix.chars() {
        let current = chars.next()?;
        if !same_letter(current, expected, locale) {
            return None;
        }
    }
    Some(value.len() - chars.as_str().len())
}

pub(super) fn strip_folded_suffix<'a>(
    value: &'a str,
    suffix: &str,
    locale: CharacterLocale,
) -> Option<&'a str> {
    let mut chars = value.chars();
    for c in suffix.chars().rev() {
        if !same_letter(chars.next_back()?, c, locale) {
            return None;
        }
    }
    Some(chars.as_str())
}

pub(crate) fn capitalize(mut text: String, locale: crate::session::CharacterLocale) -> String {
    use crate::session::CharacterLocale;
    if locale == CharacterLocale::Ascii || (text.is_ascii() && locale != CharacterLocale::Turkic) {
        text.make_ascii_lowercase();
        if let Some(first) = text.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        return text;
    }
    localized(|| {
        text.chars()
            .enumerate()
            .map(|(i, c)| mapped_char(c, i == 0, locale))
            .collect()
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CaseDirection {
    Upper,
    Lower,
    Toggle,
}

pub(crate) fn case_chars(
    text: String,
    upper: bool,
    all: bool,
    pattern: Option<&Matcher<&str>>,
    locale: CharacterLocale,
) -> String {
    let direction = if upper {
        CaseDirection::Upper
    } else {
        CaseDirection::Lower
    };
    convert_chars(text.into(), direction, all, pattern, locale)
}

pub(crate) fn convert_chars(
    text: std::borrow::Cow<'_, str>,
    direction: CaseDirection,
    all: bool,
    pattern: Option<&Matcher<&str>>,
    locale: CharacterLocale,
) -> String {
    if pattern.is_some() {
        return super::localized(|| convert(text,direction,all,pattern,locale));
    }
    convert(text,direction,all,pattern,locale)
}

fn convert(
    mut text: std::borrow::Cow<'_, str>,
    direction: CaseDirection,
    all: bool,
    pattern: Option<&Matcher<&str>>,
    locale: CharacterLocale,
) -> String {
    if pattern.is_none()
        && (locale == CharacterLocale::Ascii
            || locale != CharacterLocale::Turkic && text.is_ascii())
    {
        let end = if all {
            text.len()
        } else {
            text.chars().next().map_or(0, char::len_utf8)
        };
        let s = &mut text.to_mut()[..end];
        match direction {
            CaseDirection::Upper => s.make_ascii_uppercase(),
            CaseDirection::Lower => s.make_ascii_lowercase(),
            CaseDirection::Toggle => {
                // Only ASCII letters change, preserving every UTF-8 boundary.
                for b in unsafe { s.as_bytes_mut() } {
                    if b.is_ascii_alphabetic() {
                        *b ^= 0x20;
                    }
                }
            }
        }
        return text.into_owned();
    }
    let utf8 = utf8_locale();
    let mut recent = [('\0', '\0'); 16];
    let reuse = all && text.len() >= 256;
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    for c in chars.by_ref() {
        let mut buf = [0; 4];
        let eligible = pattern.is_none_or(|p| p.matches(c.encode_utf8(&mut buf)));
        let upper = match direction {
            CaseDirection::Upper => true,
            CaseDirection::Lower => false,
            CaseDirection::Toggle => !c.is_uppercase(),
        };
        out.push(if eligible {
            if c.is_ascii() && locale != CharacterLocale::Turkic {
                if upper {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            } else if reuse {
                let slot = &mut recent[c as usize % 16];
                if slot.0 != c {
                    *slot = (c, mapped_in(c, upper, locale, utf8));
                }
                slot.1
            } else {
                mapped_in(c, upper, locale, utf8)
            }
        } else {
            c
        });
        if !all {
            break;
        }
    }
    out.push_str(chars.as_str());
    out
}
