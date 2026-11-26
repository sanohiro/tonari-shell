use super::*;

pub(super) fn mapped_char(c: char, upper: bool, locale: CharacterLocale) -> char {
    if c.is_ascii() && !matches!(locale, CharacterLocale::Turkic) {
        return if upper {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        };
    }
    unsafe extern "C" {
        fn towupper_l(c: u32, locale: libc::locale_t) -> u32;
        fn towlower_l(c: u32, locale: libc::locale_t) -> u32;
    }
    // The shell may start in C and switch to UTF-8. Select a UTF-8 ctype
    // explicitly without changing the process locale or loading Unicode tables.
    static UTF8: OnceLock<usize> = OnceLock::new();
    let utf8 = *UTF8.get_or_init(|| {
        for name in [c"C.UTF-8", c"en_US.UTF-8", c"UTF-8"] {
            let locale = unsafe {
                libc::newlocale(libc::LC_CTYPE_MASK, name.as_ptr(), std::ptr::null_mut())
            };
            if !locale.is_null() {
                return locale as usize;
            }
        }
        0
    }) as libc::locale_t;
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

pub(crate) fn toggle_case(
    mut text: String,
    all: bool,
    filter: Option<&Matcher<&str>>,
    locale: crate::session::CharacterLocale,
) -> String {
    use crate::session::CharacterLocale;
    if filter.is_none()
        && (locale == CharacterLocale::Ascii
            || text.is_ascii() && locale != CharacterLocale::Turkic)
    {
        let len = if all { text.len() } else { text.len().min(1) };
        // Only ASCII letters change. All UTF-8 continuation and leading bytes stay intact.
        for byte in &mut unsafe { text.as_bytes_mut() }[..len] {
            if byte.is_ascii_alphabetic() {
                *byte ^= 0x20;
            }
        }
        return text;
    }
    localized(|| {
        text.chars()
            .enumerate()
            .map(|(i, c)| {
                let mut buffer = [0; 4];
                if (all || i == 0) && filter.is_none_or(|p| p.matches(c.encode_utf8(&mut buffer))) {
                    mapped_char(c, !c.is_uppercase(), locale)
                } else {
                    c
                }
            })
            .collect()
    })
}

pub(crate) fn case_chars(
    mut text: String,
    upper: bool,
    all: bool,
    pattern: Option<&Matcher<&str>>,
    locale: crate::session::CharacterLocale,
) -> String {
    use crate::session::CharacterLocale;
    if pattern.is_none()
        && (matches!(locale, CharacterLocale::Ascii)
            || text.is_ascii() && !matches!(locale, CharacterLocale::Turkic))
    {
        let end = if all {
            text.len()
        } else {
            text.chars().next().map_or(0, char::len_utf8)
        };
        if upper {
            text[..end].make_ascii_uppercase();
        } else {
            text[..end].make_ascii_lowercase();
        }
        return text;
    }
    localized(|| {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars();
        for c in chars.by_ref() {
            let mut buf = [0; 4];
            let eligible = pattern.is_none_or(|p| p.matches(c.encode_utf8(&mut buf)));
            out.push(if eligible {
                mapped_char(c, upper, locale)
            } else {
                c
            });
            if !all {
                break;
            }
        }
        out.push_str(chars.as_str());
        out
    })
}
