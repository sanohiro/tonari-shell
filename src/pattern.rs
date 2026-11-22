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

#[derive(Clone, Copy, Default)]
pub struct GlobOptions {
    pub dotglob: bool,
    pub nullglob: bool,
    pub globstar: bool,
}

pub(crate) fn pathnames(pattern: &str, options: GlobOptions) -> Vec<String> {
    let mut names = Vec::new();
    localized(|| paths(pattern, options, &mut names));
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
fn paths(pattern: &str, options: GlobOptions, fields: &mut Vec<String>) {
    let mut paths = vec![String::new()];
    let mut rest = pattern;
    let mut known_existing = false;
    let mut expanded = false;
    while !rest.is_empty() {
        known_existing = false;
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
        if options.globstar && part == "**" {
            while let Some(after) = rest.trim_start_matches('/').strip_prefix("**") {
                if !(after.is_empty() || after.starts_with('/')) {
                    break;
                }
                expanded = true;
                rest = after;
            }
            let trailing = !rest.is_empty() && rest.trim_start_matches('/').is_empty();
            rest = rest.trim_start_matches('/');
            let mut found = Vec::new();
            for prefix in paths {
                if std::path::Path::new(if prefix.is_empty() { "." } else { &prefix }).is_dir() {
                    if !rest.is_empty() || !prefix.is_empty() {
                        let root = if rest.is_empty() && !trailing && expanded && prefix != "/" {
                            prefix.trim_end_matches('/')
                        } else {
                            &prefix
                        };
                        found.push(root.to_owned());
                    }
                    descendants(
                        prefix,
                        rest.is_empty(),
                        trailing,
                        options.dotglob,
                        &mut found,
                    );
                }
            }
            expanded = true;
            found.sort_unstable();
            found.dedup();
            paths = found;
            known_existing = rest.is_empty();
            continue;
        }
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
                if unsafe {
                    libc::fnmatch(
                        part.as_ptr(),
                        name.as_ptr(),
                        if options.dotglob { 0 } else { libc::FNM_PERIOD },
                    )
                } == 0
                {
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
        expanded = true;
        paths = matched;
        known_existing = true;
    }
    // A literal suffix must exist. A trailing slash asks the OS for a directory.
    let mut ordered: Vec<_> = paths
        .into_iter()
        .filter(|path| known_existing || fs::symlink_metadata(path).is_ok())
        .filter_map(|path| CString::new(path).ok())
        .collect();
    ordered.sort_unstable_by(|a, b| {
        let order = unsafe { libc::strcoll(a.as_ptr(), b.as_ptr()) };
        order.cmp(&0).then_with(|| CStr::cmp(a, b))
    });
    fields.extend(ordered.into_iter().map(|s| s.into_string().unwrap()));
}

// The recursive component consumes zero or more directories. A symlink to a
// directory can appear in a terminal **/ listing, but is never traversed here.
fn descendants(
    prefix: String,
    terminal: bool,
    trailing: bool,
    dotglob: bool,
    found: &mut Vec<String>,
) {
    let prefixed = !prefix.is_empty();
    let mut pending = vec![prefix];
    while let Some(dir) = pending.pop() {
        let base = if dir.is_empty() { "." } else { &dir };
        let Ok(entries) = fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !dotglob && name.starts_with('.') {
                continue;
            }
            let path = format!("{dir}{name}");
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let subdir = format!("{path}/");
                found.push(if terminal && !trailing {
                    path
                } else {
                    subdir.clone()
                });
                pending.push(subdir);
            } else if terminal && !trailing {
                found.push(path);
            } else if (terminal || prefixed) && kind.is_symlink() && entry.path().is_dir() {
                found.push(format!("{path}/"));
            }
        }
    }
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

    pub fn finish(self, fields: &mut Vec<String>, globbing: Option<GlobOptions>) {
        if let Some((options, pattern)) = globbing.zip(self.pattern).filter(|(_, p)| has_magic(p)) {
            let count = fields.len();
            localized(|| paths(&pattern, options, fields));
            if fields.len() > count || options.nullglob {
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
        field.finish(&mut words, Some(GlobOptions::default()));
        assert_eq!(words, [format!("{root}word.rs")]);

        let mut field = Field::default();
        field.push(root, false);
        field.push("*", false);
        field.push("-missing-?", true);
        field.finish(&mut words, Some(GlobOptions::default()));
        assert_eq!(words[1], format!("{root}*-missing-?"));
    }
}
