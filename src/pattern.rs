mod bracket;
pub(crate) mod directory;
pub(crate) mod extended;
mod glob_locale;
mod order;

use crate::session::CharacterLocale;
use directory::visit_names;
use order::Collation;
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
    let before = unsafe { libc::uselocale(locale) };
    let result = work();
    if !before.is_null() {
        unsafe {
            libc::uselocale(before);
        }
    }
    result
}

// A star consumes the gap between literal pieces. Bracket expressions,
// question marks and quoting use the locale-aware matcher.
#[derive(Debug)]
pub(crate) enum Matcher<T> {
    Literal(T),
    Star(T, usize),
    Stars(T, usize, usize),
    Native(CString),
    Bracket(bracket::BracketPattern, T),
    Invalid,
}

impl<T: AsRef<str>> Matcher<T> {
    pub(crate) fn new(pattern: T) -> Self {
        let text = pattern.as_ref();
        if !text.bytes().any(|b| matches!(b, b'?' | b'[' | b'\\' | 0)) {
            match text.split_once('*') {
                None => return Self::Literal(pattern),
                Some((left, right)) => {
                    let star = left.len();
                    if let Some(last) = right.rfind('*') {
                        return Self::Stars(pattern, star, star + 1 + last);
                    }
                    return Self::Star(pattern, star);
                }
            }
        }
        if text.contains('[') && bracket::needs_parser(text.as_bytes()) {
            return bracket::BracketPattern::new(text.as_bytes())
                .map_or(Self::Invalid, |compiled| Self::Bracket(compiled, pattern));
        }
        CString::new(text).map_or(Self::Invalid, Self::Native)
    }

    // Call with the matching locale active for Native. Directory entries also
    // supply the NUL-terminated view of the same bytes for fnmatch.
    fn accepts(&self, value: &[u8], period: bool, c_name: Option<&CStr>) -> bool {
        match self {
            Self::Literal(literal) => value == literal.as_ref().as_bytes(),
            Self::Star(text, star) => {
                let (left, tail) = text.as_ref().as_bytes().split_at(*star);
                !(period && left.is_empty() && value.first() == Some(&b'.'))
                    && value
                        .strip_prefix(left)
                        .is_some_and(|rest| rest.ends_with(&tail[1..]))
            }
            Self::Stars(text, first, last) => {
                let pattern = text.as_ref();
                let head = &pattern[..*first];
                if period && head.is_empty() && value.first() == Some(&b'.') {
                    return false;
                }
                let Some(gap) = value
                    .strip_prefix(head.as_bytes())
                    .and_then(|rest| rest.strip_suffix(&pattern.as_bytes()[last + 1..]))
                else {
                    return false;
                };
                let Ok(mut gap) = std::str::from_utf8(gap) else {
                    return false;
                };
                for part in pattern[first + 1..*last]
                    .split('*')
                    .filter(|p| !p.is_empty())
                {
                    let Some((_, after)) = gap.split_once(part) else {
                        return false;
                    };
                    gap = after;
                }
                true
            }
            Self::Native(pattern) => {
                let test = |name: &CStr| unsafe {
                    libc::fnmatch(
                        pattern.as_ptr(),
                        name.as_ptr(),
                        if period { libc::FNM_PERIOD } else { 0 },
                    ) == 0
                };
                match c_name {
                    Some(name) => test(name),
                    None => CString::new(value).is_ok_and(|name| test(&name)),
                }
            }
            Self::Bracket(pattern, _) => pattern.accepts(value, period),
            Self::Invalid => false,
        }
    }

    pub(crate) fn source(&self) -> &str {
        match self {
            Self::Literal(s) | Self::Star(s, _) | Self::Stars(s, ..) | Self::Bracket(_, s) => {
                s.as_ref()
            }
            Self::Native(s) => s.to_str().unwrap(),
            Self::Invalid => "",
        }
    }

    pub(crate) fn insensitive(&self, value: &str, extended: bool, locale: CharacterLocale) -> bool {
        if matches!(self, Self::Invalid) {
            return false;
        }
        if !extended && let Self::Literal(text) = self {
            return folded_prefix(value, text.as_ref(), locale) == Some(value.len());
        }
        extended::insensitive_match(value, self.source(), extended, locale)
    }

    pub(crate) fn matches(&self, value: &str) -> bool {
        if value.contains('\0') {
            return false;
        }
        if matches!(self, Self::Native(_) | Self::Bracket(..)) {
            localized(|| self.accepts(value.as_bytes(), false, None))
        } else {
            self.accepts(value.as_bytes(), false, None)
        }
    }
}

pub(crate) fn bracket_end(after_open: &str) -> Option<usize> {
    bracket::class_end(after_open.as_bytes())
}

pub fn matches(value: &str, pattern: &str) -> bool {
    Matcher::new(pattern).matches(value)
}

fn mapped_char(c: char, upper: bool, locale: CharacterLocale) -> char {
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

fn same_letter(left: char, right: char, locale: CharacterLocale) -> bool {
    if left == right {
        return true;
    }
    if left.is_ascii() && right.is_ascii() && !matches!(locale, CharacterLocale::Turkic) {
        return left.eq_ignore_ascii_case(&right);
    }
    mapped_char(left, false, locale) == mapped_char(right, false, locale)
}

pub(super) fn folded_prefix(value: &str, prefix: &str, locale: CharacterLocale) -> Option<usize> {
    let mut chars = value.chars();
    for expected in prefix.chars() {
        let current = chars.next()?;
        if !same_letter(current, expected, locale) {
            return None;
        }
    }
    Some(value.len() - chars.as_str().len())
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

pub fn trim(
    mut value: String,
    pattern: &str,
    prefix: bool,
    longest: bool,
    extglob: bool,
) -> String {
    if let Some(cut) = trim_boundary(&value, pattern, prefix, longest, extglob) {
        if prefix {
            value.drain(..cut);
        } else {
            value.truncate(cut);
        }
    }
    value
}

fn trim_boundary(
    value: &str,
    pattern: &str,
    prefix: bool,
    longest: bool,
    extglob: bool,
) -> Option<usize> {
    // A star at the end being removed can absorb everything up to a literal
    // delimiter. The nearest or farthest occurrence gives the exact cut.
    let star = if prefix {
        pattern.strip_prefix('*')
    } else {
        pattern.strip_suffix('*')
    };
    let literal = star.unwrap_or(pattern);
    let group = extglob.then(|| extended::Pattern::new(pattern)).flatten();
    let plain = group.is_none()
        && !literal
            .bytes()
            .any(|c| matches!(c, b'*' | b'?' | b'[' | b'\\' | 0))
        && !value.contains('\0');
    if plain {
        if star.is_some() {
            let offset = if prefix == longest {
                value.rfind(literal)
            } else {
                value.find(literal)
            }?;
            return Some(if prefix {
                offset + literal.len()
            } else {
                offset
            });
        }
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
        group.as_ref().map_or_else(
            || matches(candidate, pattern),
            |g| g.accepts(candidate, false),
        )
    };
    if prefix == longest {
        cuts.rfind(matches_at)
    } else {
        cuts.find(matches_at)
    }
}

pub fn quote_into(output: &mut String, text: &str) {
    quote_pattern(output, text, false);
}

pub(crate) fn quote_pattern(output: &mut String, text: &str, extended: bool) {
    // The first literal segment cannot be inside a bracket expression.
    // Later segments can follow active syntax, for example [a"-"z].
    let may_be_in_bracket = !output.is_empty();
    let symbol = output.rsplit_once('[').is_some_and(|(prefix, rest)| {
        !prefix.is_empty() && rest.starts_with(['.', '=']) && !rest.contains(']')
    });
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '?' | '[')
            || extended && matches!(c, '@' | '+' | '!' | '(' | ')' | '|')
            || symbol
            || (matches!(c, ':' | '.' | '=') && output.ends_with('['))
            || may_be_in_bracket && matches!(c, ']' | '-' | '!' | '^')
        {
            output.push('\\');
        }
        output.push(c);
    }
}

pub(crate) fn has_magic(text: &str, pathname: bool) -> bool {
    let mut bracket = false;
    let mut bytes = text.bytes();
    while let Some(c) = bytes.next() {
        match c {
            b'\\' => {
                bytes.next();
            }
            b'*' | b'?' => return true,
            b'[' => bracket = true,
            b']' if bracket => return true,
            b'/' if pathname => bracket = false,
            _ => {}
        }
    }
    false
}

#[derive(Clone)]
pub struct GlobOptions {
    pub dotglob: bool,
    pub nullglob: bool,
    pub failglob: bool,
    pub globstar: bool,
    pub extglob: bool,
    pub globskipdots: bool,
    ignore: Option<std::rc::Rc<[String]>>,
}

impl Default for GlobOptions {
    fn default() -> Self {
        Self {
            dotglob: false,
            nullglob: false,
            failglob: false,
            globstar: false,
            extglob: false,
            globskipdots: true,
            ignore: None,
        }
    }
}

impl GlobOptions {
    pub(crate) fn set_ignore(&mut self, value: Option<&str>) {
        self.ignore = None;
        let Some(value) = value else {
            self.dotglob = false;
            return;
        };
        if value.is_empty() {
            return;
        }
        self.dotglob = true;
        let mut patterns = Vec::new();
        let mut begin = 0;
        let mut depth = 0usize;
        let mut chars = value.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => {
                    chars.next();
                }
                '[' => {
                    if let Some(end) = bracket_end(&value[i + 1..]) {
                        let after = i + 2 + end;
                        while chars.clone().next().is_some_and(|(j, _)| j < after) {
                            chars.next();
                        }
                    }
                }
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ':' if depth == 0 => {
                    if begin != i {
                        patterns.push(value[begin..i].to_owned());
                    }
                    begin = i + 1;
                }
                _ => {}
            }
        }
        if begin != value.len() {
            patterns.push(value[begin..].to_owned());
        }
        self.ignore = Some(patterns.into());
    }
}

pub(crate) fn pathnames(
    pattern: &str,
    options: &GlobOptions,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut names = Vec::new();
    glob_locale::run(lookup, |order| paths(pattern, options, &mut names, order));
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
fn paths(pattern: &str, options: &GlobOptions, fields: &mut Vec<String>, collation: Collation) {
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
        let extended = options
            .extglob
            .then(|| extended::Pattern::new(part))
            .flatten();
        if extended.is_none() && !has_magic(part, true) {
            let literal = unescape(part);
            for prefix in &mut paths {
                prefix.push_str(&literal);
            }
            continue;
        }
        let matcher = Matcher::new(part);
        let mut matched = Vec::new();
        for prefix in paths {
            let dir = if prefix.is_empty() { "." } else { &prefix };
            visit_names(
                dir,
                !rest.is_empty(),
                options.globskipdots || options.ignore.is_some(),
                |raw, _| {
                    let period = !options.dotglob || matches!(raw.to_bytes(), b"." | b"..");
                    let matches = match &extended {
                        Some(group) => raw.to_str().is_ok_and(|s| group.accepts(s, period)),
                        None => matcher.accepts(raw.to_bytes(), period, Some(raw)),
                    };
                    if !matches {
                        return;
                    }
                    let Ok(name) = raw.to_str() else { return };
                    let mut path = String::with_capacity(
                        prefix.len()
                            + name.len()
                            + usize::from(collation != Collation::Bytes || !rest.is_empty()),
                    );
                    path.push_str(&prefix);
                    path.push_str(name);
                    matched.push(path);
                },
            );
        }
        if matched.is_empty() {
            return;
        }
        expanded = true;
        paths = matched;
        known_existing = true;
    }
    // A literal suffix must exist. A trailing slash asks the OS for a directory.
    if !known_existing {
        paths.retain(|path| fs::symlink_metadata(path).is_ok());
    }
    if let Some(patterns) = &options.ignore {
        for pattern in patterns.iter() {
            // GLOBIGNORE tests the entire displayed pathname. An extended
            // alternative may contain a slash or match one with a bracket class.
            let matcher = extended::Pattern::parse(pattern, options.extglob);
            paths.retain(|path| !matcher.pathname(path));
        }
    }
    order::append(paths, fields, collation);
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
        visit_names(base, !terminal || trailing, true, |raw, kind| {
            if !dotglob && raw.to_bytes().first() == Some(&b'.') {
                return;
            }
            let Ok(name) = raw.to_str() else { return };
            let mut path = String::with_capacity(dir.len() + name.len() + 1);
            path.push_str(&dir);
            path.push_str(name);
            let (is_dir, is_link) = if kind == libc::DT_UNKNOWN {
                let Ok(info) = fs::symlink_metadata(&path) else {
                    return;
                };
                (info.is_dir(), info.is_symlink())
            } else {
                (kind == libc::DT_DIR, kind == libc::DT_LNK)
            };
            if is_dir {
                if terminal && !trailing {
                    found.push(path.clone());
                    path.push('/');
                } else {
                    path.push('/');
                    found.push(path.clone());
                }
                pending.push(path);
            } else if terminal && !trailing {
                found.push(path);
            } else if (terminal || prefixed) && is_link && std::path::Path::new(&path).is_dir() {
                path.push('/');
                found.push(path);
            }
        });
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
                .any(|b| matches!(b, b'*' | b'?' | b'[' | b'\\' | b'('))
        {
            let mut pattern = String::new();
            quote_pattern(&mut pattern, &self.text, true);
            self.pattern = Some(pattern);
        }
        if let Some(pattern) = &mut self.pattern {
            if active {
                pattern.push_str(text);
            } else {
                quote_pattern(pattern, text, true);
            }
        }
        self.text.push_str(text);
    }

    pub fn finish(
        self,
        fields: &mut Vec<String>,
        globbing: Option<&GlobOptions>,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), crate::expansion::Error> {
        if let Some((options, pattern)) = globbing
            .zip(self.pattern)
            .filter(|(options, p)| has_magic(p, true) || options.extglob && extended::has_group(p))
        {
            let count = fields.len();
            glob_locale::run(lookup, |order| paths(&pattern, options, fields, order));
            if fields.len() > count {
                return Ok(());
            }
            if options.failglob {
                return Err(crate::expansion::Error::discard(format!(
                    "no match: {}",
                    self.text
                )));
            }
            if options.nullglob {
                return Ok(());
            }
        }
        fields.push(self.text);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_and_star_patterns_agree_with_native_matching() {
        let values = [
            "",
            "a",
            "aba",
            "ababa",
            ".env",
            "a.b",
            "雪🌱é",
            "雪🌱e\u{301}",
            "a/b",
            "a*b",
        ];
        localized(|| {
            for source in [
                "",
                "*",
                "aba",
                "ab*ba",
                "a*",
                "*a",
                "*env",
                ".*",
                "a*b",
                "雪*é",
                "雪*e\u{301}",
                "?*",
                "a**b",
                "***",
                "ab**ba",
                "*aba*ba",
                "*a*a*a*",
                "*b***a*b*",
                "*雪*é",
                "雪*🌱*",
                "*.*",
                ".**v",
                r"a\*b",
                "[!a]*",
                ".??*",
            ] {
                let matcher = Matcher::new(source);
                let native = CString::new(source).unwrap();
                for value in values {
                    let name = CString::new(value).unwrap();
                    for period in [false, true] {
                        let flags = if period { libc::FNM_PERIOD } else { 0 };
                        let expected =
                            unsafe { libc::fnmatch(native.as_ptr(), name.as_ptr(), flags) == 0 };
                        assert_eq!(
                            matcher.accepts(value.as_bytes(), period, None),
                            expected,
                            "{source:?} {value:?} {flags}"
                        );
                    }
                }
            }
        });
    }

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
                            trim_boundary(text, pattern, prefix, longest, false),
                            expected.copied(),
                            "{text:?} {pattern:?} prefix={prefix} longest={longest}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn literal_prefix_keeps_single_star_matcher_with_punctuation() {
        let mut pattern = String::new();
        quote_into(&mut pattern, "item-!^]雪");
        pattern.push_str("*.txt");
        let matcher = Matcher::new(&pattern);
        assert!(matches!(matcher, Matcher::Star(..)));
        assert!(matcher.matches("item-!^]雪001.txt"));
        let mut range = String::from("[a");
        quote_into(&mut range, "-");
        range.push_str("z]");
        assert!(matches("-", &range));
        assert!(!matches("m", &range));
        assert!(matches("q--", r"[!a\-z][\-!][b\-]"));
        assert!(!matches("a--", r"[!a\-z][\-!][b\-]"));
        assert!(matches("m", "[a-z]"));
        assert!(matches("[a-z]", r"\[a\-z]"));
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
        field
            .finish(&mut words, Some(&GlobOptions::default()), &|_| None)
            .unwrap();
        assert_eq!(words, [format!("{root}word.rs")]);

        let mut field = Field::default();
        field.push(root, false);
        field.push("*", false);
        field.push("-missing-?", true);
        field
            .finish(&mut words, Some(&GlobOptions::default()), &|_| None)
            .unwrap();
        assert_eq!(words[1], format!("{root}*-missing-?"));
    }
}
