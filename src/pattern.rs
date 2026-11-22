use std::ffi::{CStr, CString};
use std::fs;
use std::sync::OnceLock;

// libc starts in the C locale. Load the caller's character and collation
// settings only when a pattern needs them, and restore the thread afterwards.
pub(crate) fn localized<T>(work: impl FnOnce() -> T) -> T {
    static LOCALE: OnceLock<usize> = OnceLock::new();
    let locale = *LOCALE.get_or_init(|| unsafe {
        libc::newlocale(
            libc::LC_CTYPE_MASK | libc::LC_COLLATE_MASK,
            c"".as_ptr(),
            std::ptr::null_mut(),
        ) as usize
    }) as libc::locale_t;
    if locale.is_null() {
        return work();
    }
    let previous = unsafe { libc::uselocale(locale) };
    let result = work();
    if !previous.is_null() {
        unsafe { libc::uselocale(previous) };
    }
    result
}

pub fn matches(value: &str, pattern: &str) -> bool {
    let (Ok(value), Ok(pattern)) = (CString::new(value), CString::new(pattern)) else {
        return false;
    };
    localized(|| unsafe { libc::fnmatch(pattern.as_ptr(), value.as_ptr(), 0) == 0 })
}

pub fn trim(mut value: String, pattern: &str, prefix: bool, longest: bool) -> String {
    if let Some(cut) = trim_boundary(&value, pattern, prefix, longest) {
        if prefix {
            value.drain(..cut);
        } else {
            value.truncate(cut);
        }
    }
    value
}

fn trim_boundary(value: &str, pattern: &str, prefix: bool, longest: bool) -> Option<usize> {
    // A star at the end being removed can absorb everything up to a literal
    // delimiter. The nearest or farthest occurrence gives the exact cut.
    let star = if prefix {
        pattern.strip_prefix('*')
    } else {
        pattern.strip_suffix('*')
    };
    let literal = star.unwrap_or(pattern);
    let plain = !literal
        .bytes()
        .any(|c| matches!(c, b'*' | b'?' | b'[' | b'\\' | 0))
        && !value.contains('\0');
    if plain && star.is_none() {
        return if prefix {
            value.starts_with(literal).then_some(literal.len())
        } else {
            value.strip_suffix(literal).map(str::len)
        };
    }
    // Try only UTF-8 boundaries, including the empty prefix or suffix.
    let mut cuts = value
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(value.len()));
    let matches_at = |&i: &usize| {
        let candidate = if prefix { &value[..i] } else { &value[i..] };
        if plain {
            if prefix {
                candidate.ends_with(literal)
            } else {
                candidate.starts_with(literal)
            }
        } else {
            matches(candidate, pattern)
        }
    };
    if prefix == longest {
        cuts.rfind(matches_at)
    } else {
        cuts.find(matches_at)
    }
}

pub fn quote_into(output: &mut String, text: &str) {
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']' | '-' | '!' | '^') {
            output.push('\\');
        }
        output.push(c);
    }
}

pub(crate) fn has_magic(text: &str) -> bool {
    let mut bytes = text.bytes();
    while let Some(c) = bytes.next() {
        match c {
            b'\\' => {
                bytes.next();
            }
            b'*' | b'?' | b'[' => return true,
            _ => {}
        }
    }
    false
}

pub(crate) fn pathnames(pattern: &str) -> Vec<String> {
    let mut names = Vec::new();
    localized(|| paths(pattern, &mut names));
    names
}

pub(crate) fn unescape(component: &str) -> String {
    let mut literal = String::with_capacity(component.len());
    let mut chars = component.chars();
    while let Some(c) = chars.next() {
        literal.push(if c == '\\' {
            chars.next().unwrap_or(c)
        } else {
            c
        });
    }
    literal
}

// Walk by component so POSIX character classes have the same fnmatch
// semantics in argv and in case patterns, including on Darwin.
fn paths(pattern: &str, fields: &mut Vec<String>) {
    let mut paths = vec![String::new()];
    let mut rest = pattern;
    while !rest.is_empty() {
        let slash_count = rest.bytes().take_while(|&b| b == b'/').count();
        if slash_count > 0 {
            for prefix in &mut paths {
                prefix.push_str(&rest[..slash_count]);
            }
            rest = &rest[slash_count..];
            continue;
        }
        let end = rest.find('/').unwrap_or(rest.len());
        let part = &rest[..end];
        rest = &rest[end..];
        if !has_magic(part) {
            let literal = unescape(part);
            for prefix in &mut paths {
                prefix.push_str(&literal);
            }
            continue;
        }
        let Ok(part) = CString::new(part) else { return };
        let mut matched = Vec::new();
        for prefix in paths {
            let dir = if prefix.is_empty() { "." } else { &prefix };
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let Ok(name) = CString::new(name) else {
                    continue;
                };
                if unsafe { libc::fnmatch(part.as_ptr(), name.as_ptr(), libc::FNM_PERIOD) } == 0 {
                    // read_dir omits "." and "..", matching Bash 5.2+ globskipdots.
                    let mut path = String::with_capacity(prefix.len() + name.as_bytes().len());
                    path.push_str(&prefix);
                    path.push_str(name.to_str().unwrap());
                    matched.push(path);
                }
            }
        }
        if matched.is_empty() {
            return;
        }
        paths = matched;
    }
    // A literal suffix must exist. A trailing slash asks the OS for a directory.
    let mut ordered: Vec<_> = paths
        .into_iter()
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .filter_map(|path| CString::new(path).ok())
        .collect();
    ordered.sort_unstable_by(|a, b| {
        let order = unsafe { libc::strcoll(a.as_ptr(), b.as_ptr()) };
        order.cmp(&0).then_with(|| CStr::cmp(a, b))
    });
    fields.extend(ordered.into_iter().map(|s| s.into_string().unwrap()));
}

// Keep a matching expression alongside the actual argument only once active
// pattern syntax appears. The original argument survives a failed match.
#[derive(Default)]
pub struct Field {
    text: String,
    pattern: Option<String>,
}

impl Field {
    pub fn push(&mut self, text: &str, active: bool) {
        if active
            && self.pattern.is_none()
            && text
                .bytes()
                .any(|b| matches!(b, b'*' | b'?' | b'[' | b'\\'))
        {
            let mut pattern = String::new();
            quote_into(&mut pattern, &self.text);
            self.pattern = Some(pattern);
        }
        if let Some(pattern) = &mut self.pattern {
            if active {
                pattern.push_str(text);
            } else {
                quote_into(pattern, text);
            }
        }
        self.text.push_str(text);
    }

    pub fn finish(self, fields: &mut Vec<String>, globbing: bool) {
        if let Some(pattern) = self
            .pattern
            .filter(|pattern| globbing && has_magic(pattern))
        {
            let count = fields.len();
            localized(|| paths(&pattern, fields));
            if fields.len() > count {
                return;
            }
        }
        fields.push(self.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_boundaries_agree_with_glob_matching() {
        for text in ["", "aaaa", "/a/雪/雪", ".log", "a.log.log", "a*b"] {
            for pattern in [
                "", "*", ".log", "*aa", "aa*", "*/", "/*", "*雪", "雪*", "a*z", "*?", "[a]*",
                r"\*", "a\\*b",
            ] {
                for prefix in [true, false] {
                    let cuts: Vec<_> = text
                        .char_indices()
                        .map(|(i, _)| i)
                        .chain(std::iter::once(text.len()))
                        .filter(|&i| matches(if prefix { &text[..i] } else { &text[i..] }, pattern))
                        .collect();
                    for longest in [true, false] {
                        let expected = if prefix == longest {
                            cuts.last()
                        } else {
                            cuts.first()
                        };
                        assert_eq!(
                            trim_boundary(text, pattern, prefix, longest),
                            expected.copied(),
                            "{text:?} {pattern:?} prefix={prefix} longest={longest}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn quoted_wildcards_remain_literal_in_case_patterns() {
        let mut pattern = String::new();
        quote_into(&mut pattern, "a[?]*");
        pattern.push_str("[[:alpha:]]");
        assert!(matches("a[?]*z", &pattern));
        assert!(!matches("a[?]*4", &pattern));
        assert!(matches(".dir/file", "*"));
    }

    #[test]
    fn pathname_matches_use_character_classes_and_preserve_an_unmatched_word() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src/");
        let mut field = Field::default();
        field.push(root, false);
        field.push("[[:alpha:]]ord.rs", true);
        let mut words = Vec::new();
        field.finish(&mut words, true);
        assert_eq!(words, [format!("{root}word.rs")]);

        let mut field = Field::default();
        field.push(root, false);
        field.push("*", false);
        field.push("-missing-?", true);
        field.finish(&mut words, true);
        assert_eq!(words[1], format!("{root}*-missing-?"));
    }
}
