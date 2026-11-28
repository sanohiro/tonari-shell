mod bracket;
pub(crate) use bracket::portable_symbol;
pub(crate) mod directory;
pub(crate) mod extended;
mod glob_locale;
pub(crate) use glob_locale::{Cached as PatternLocale, sync_ranges};

pub(crate) fn session_locale(session: &crate::Session) -> Option<std::rc::Rc<PatternLocale>> {
    thread_local! { static INHERITED: std::cell::OnceCell<Option<std::rc::Rc<PatternLocale>>> = const { std::cell::OnceCell::new() }; }
    if session.pattern_locale_overridden() {
        if let Some(all) = session.scalar_setting("LC_ALL").filter(|s| !s.is_empty()) {
            return glob_locale::resolve_names(&all, &all);
        }
        let ctype = session.locale_category("LC_CTYPE");
        let collate = session.locale_category("LC_COLLATE");
        glob_locale::resolve_names(ctype.as_deref().unwrap_or("C"), collate.as_deref().unwrap_or("C"))
    } else {
        INHERITED.with(|cell| {
            cell.get_or_init(|| glob_locale::resolve(&|key| std::env::var(key).ok()))
                .clone()
        })
    }
}

pub(crate) fn text_order(left: &str, right: &str, session: &crate::Session) -> std::cmp::Ordering {
    let locale = (session.shopt.compat > 40).then(|| session_locale(session)).flatten();
    if let Some(locale) = locale && !matches!(locale.collate.as_str(), "C" | "POSIX" | "C.UTF-8" | "C.utf8")
        && let (Ok(left), Ok(right)) = (std::ffi::CString::new(left), std::ffi::CString::new(right))
        && let Some(_locale) = locale.enter() {
        return unsafe { libc::strcoll(left.as_ptr(), right.as_ptr()) }.cmp(&0);
    }
    left.cmp(right)
}

impl PatternLocale {
    pub(crate) fn enter(&self) -> Option<LocaleGuard> {
        LocaleGuard::enter(self.handle)
    }
}
mod order;

use crate::session::CharacterLocale;
use directory::visit_names;
use order::Collation;
pub(crate) use order::sorted_names;
use std::ffi::{CStr, CString};
use std::fs;
use std::sync::OnceLock;

// libc starts in the C locale. Load the caller's character and collation
// settings only when a pattern needs them, and restore the thread afterwards.
pub(crate) fn localized<T>(work: impl FnOnce() -> T) -> T {
    if EXPLICIT_LOCALE.get() {
        return work();
    }
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
    let _locale = LocaleGuard::enter(locale);
    work()
}

thread_local! {
    static EXPLICIT_LOCALE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(crate) struct LocaleGuard(libc::locale_t, bool);

impl LocaleGuard {
    fn enter(locale: libc::locale_t) -> Option<Self> {
        let previous = unsafe { libc::uselocale(locale) };
        if previous.is_null() {
            None
        } else {
            Some(Self(previous, EXPLICIT_LOCALE.replace(true)))
        }
    }
}

impl Drop for LocaleGuard {
    fn drop(&mut self) {
        unsafe {
            libc::uselocale(self.0);
            EXPLICIT_LOCALE.set(self.1);
        }
    }
}

// A star consumes the gap between literal pieces. Bracket expressions,
// question marks and quoting use the locale-aware matcher.
#[derive(Debug)]
pub(crate) enum Matcher<T> {
    Literal(T),
    Star(T, usize),
    Stars(T, usize, usize),
    AsciiList(T, [u64; 2]),
    Native(CString),
    Bracket(Box<bracket::BracketPattern>, T),
    Invalid,
}

// A wildcard followed by an unfinished escape cannot match. Native fnmatch
// implementations disagree when the subject itself ends in a backslash.
pub(crate) fn unfinished_star_escape(text: &str) -> bool {
    text.strip_suffix("*\\")
        .is_some_and(|prefix| prefix.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 0)
}

pub(crate) fn trailing_escape(text: &str) -> bool {
    text.ends_with('\\') && text.bytes().rev().take_while(|&b| b == b'\\').count() % 2 != 0
}

impl<T: AsRef<str>> Matcher<T> {
    pub(crate) fn new(pattern: T) -> Self {
        Self::compile(pattern, false)
    }

    fn compile(pattern: T, pathname: bool) -> Self {
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
        if unfinished_star_escape(text) {
            return Self::Invalid;
        }
        if !pathname && trailing_escape(text) {
            if !text[..text.len()-1].contains(['*','?','[','\\']) { return Self::Literal(pattern); }
            let mut quoted = text.to_owned();
            quoted.push('\\');
            return CString::new(quoted).map_or(Self::Invalid, Self::Native);
        }
        if let Some(list) = text.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
            && !list.is_empty()
            && list.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            let mut mask = [0; 2];
            for b in list.bytes() {
                mask[(b / 64) as usize] |= 1 << (b % 64);
            }
            return Self::AsciiList(pattern, mask);
        }
        if text.contains('[')
            && (bracket::needs_parser(text.as_bytes()) || pathname && bracket::simple_list(text))
        {
            return bracket::BracketPattern::new(text.as_bytes())
                .map_or(Self::Invalid, |compiled| {
                    Self::Bracket(Box::new(compiled), pattern)
                });
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
            Self::AsciiList(_, mask) => {
                matches!(value, &[b] if b.is_ascii() && mask[(b / 64) as usize] & (1 << (b % 64)) != 0)
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
            Self::AsciiList(s, _)
            | Self::Literal(s)
            | Self::Star(s, _)
            | Self::Stars(s, ..)
            | Self::Bracket(_, s) => s.as_ref(),
            Self::Native(s) => s.to_str().unwrap(),
            Self::Invalid => "",
        }
    }

    pub(crate) fn insensitive(
        &self,
        value: &str,
        extended: bool,
        session: &crate::Session,
    ) -> bool {
        if matches!(self, Self::Invalid) {
            return false;
        }
        if !extended && let Self::Literal(text) = self {
            return folded_prefix(value, text.as_ref(), session.character_locale())
                == Some(value.len());
        }
        extended::insensitive_session(value, self.source(), extended, session)
    }

    pub(crate) fn matches_session(&self, value: &str, session: &crate::Session) -> bool {
        if !matches!(self, Self::Bracket(..) | Self::Native(_)) {
            return self.matches(value);
        }
        let locale = session_locale(session);
        let _guard = locale.as_ref().and_then(|l| l.enter());
        if let Self::Bracket(compiled, source) = self
            && compiled.utf8 != bracket::BracketPattern::utf8_locale()
        {
            return bracket::BracketPattern::new(source.as_ref().as_bytes())
                .is_some_and(|p| p.accepts(value.as_bytes(), false));
        }
        self.matches(value)
    }

    pub(crate) fn matches(&self, value: &str) -> bool {
        let accepted = if matches!(self, Self::Native(_))
            || matches!(self, Self::Bracket(p, _) if p.needs_locale)
        {
            localized(|| self.accepts(value.as_bytes(), false, None))
        } else {
            self.accepts(value.as_bytes(), false, None)
        };
        accepted && memchr::memchr(0, value.as_bytes()).is_none()
    }
}

// Once a multibyte character is reached the locale-aware matcher decides
// its width and case mapping (e.g. Kelvin sign and ASCII K).
fn possible_folded_prefix(name: &[u8], prefix: &[u8]) -> bool {
    for (i, p) in prefix.iter().enumerate() {
        let Some(b) = name.get(i) else {
            return false;
        };
        if !b.is_ascii() {
            return true;
        }
        if !p.eq_ignore_ascii_case(b) {
            return false;
        }
    }
    true
}

pub(crate) fn bracket_end(after_open: &str) -> Option<usize> {
    bracket::class_end(after_open.as_bytes())
}

// A fixed-width glob consumes this many ASCII bytes. In a byte locale,
// multiple wildcard atoms can also consume a single UTF-8 scalar.
pub(crate) struct FixedWidth {
    pub(crate) width: usize,
    literals_and_questions: bool,
}

impl FixedWidth {
    pub(crate) fn parse(pattern: &str) -> Option<Self> {
        if !pattern.is_ascii() || pattern.contains('\0') {
            return None;
        }
        let mut rest = pattern.as_bytes();
        let mut width = 0;
        let mut literals_and_questions = true;
        while let Some((&atom, tail)) = rest.split_first() {
            rest = match atom {
                b'*' => return None,
                b'[' => {
                    let end = bracket::class_end(tail)?;
                    if tail[..end].windows(2).any(|p| p == b"[." || p == b"[=") {
                        return None;
                    }
                    literals_and_questions = false;
                    &tail[end + 1..]
                }
                b'\\' => {
                    literals_and_questions = false;
                    tail.get(1..)?
                }
                _ => tail,
            };
            width += 1;
        }
        Some(Self {
            width,
            literals_and_questions,
        })
    }

    pub(crate) fn literal_span<'a>(&self, pattern: &'a str) -> Option<(usize, &'a [u8])> {
        if !self.literals_and_questions {
            return None;
        }
        let bare = pattern.trim_start_matches('?');
        let literal = bare.trim_end_matches('?');
        (!literal.contains('?')).then_some((pattern.len() - bare.len(), literal.as_bytes()))
    }

    pub(crate) fn matches_ascii(
        &self,
        pattern: &str,
        candidate: &str,
        matcher: &Matcher<&str>,
    ) -> bool {
        candidate.len() == self.width
            && if self.literals_and_questions {
                pattern
                    .bytes()
                    .zip(candidate.bytes())
                    .all(|(p, c)| c != 0 && (p == b'?' || p == c))
            } else if let Matcher::Bracket(compiled, _) = matcher
                && !compiled.needs_locale
            {
                !candidate.as_bytes().contains(&0)
                    && compiled.accepts_fixed_ascii(candidate.as_bytes())
            } else {
                matcher.matches(candidate)
            }
    }
}

pub fn matches(value: &str, pattern: &str) -> bool {
    Matcher::new(pattern).matches(value)
}

mod case;
use case::mapped_char;
pub(crate) use case::{
    CaseDirection, capitalize, case_chars, convert_chars, folded_prefix, same_letter,
};

pub(crate) struct ParameterPattern<'a> {
    pub(crate) text: &'a str,
    basic: std::cell::OnceCell<Matcher<&'a str>>,
    width: std::cell::OnceCell<Option<FixedWidth>>,
    starts: std::cell::OnceCell<Matcher<String>>,
    pub(crate) extended: bool,
    pub(crate) fold: Option<crate::session::CharacterLocale>,
    pub(crate) group: Option<crate::pattern::extended::Pattern>,
}

impl<'a> ParameterPattern<'a> {
    pub(crate) fn new(
        text: &'a str,
        extended: bool,
        fold: Option<crate::session::CharacterLocale>,
    ) -> Self {
        use crate::pattern::extended::Pattern;
        let group = match fold {
            Some(locale)
                if !text.is_empty()
                    && (crate::pattern::has_magic(text, false)
                        || extended && crate::pattern::extended::has_group(text)) =>
            {
                Some(Pattern::insensitive(text, extended, locale))
            }
            _ if extended => Pattern::new(text),
            _ => None,
        };
        Self {
            text,
            basic: Default::default(),
            width: Default::default(),
            starts: Default::default(),
            extended,
            fold,
            group,
        }
    }
    pub(crate) fn basic(&self) -> &Matcher<&'a str> {
        self.basic.get_or_init(|| Matcher::new(self.text))
    }

    pub(crate) fn width(&self) -> Option<&FixedWidth> {
        self.width
            .get_or_init(|| FixedWidth::parse(self.text))
            .as_ref()
    }

    pub(crate) fn starts(&self) -> &Matcher<String> {
        self.starts
            .get_or_init(|| Matcher::new(format!("{}*", self.text)))
    }
}

pub fn trim(
    mut value: String,
    compiled: &ParameterPattern<'_>,
    prefix: bool,
    longest: bool,
) -> String {
    if let Some(cut) = trim_boundary(&value, compiled, prefix, longest) {
        if prefix {
            value.drain(..cut);
        } else {
            value.truncate(cut);
        }
    }
    value
}

pub(crate) fn trim_boundary(
    value: &str,
    compiled: &ParameterPattern<'_>,
    prefix: bool,
    longest: bool,
) -> Option<usize> {
    let pattern = compiled.text;
    if unfinished_star_escape(pattern) {
        return None;
    }
    let group = compiled.group.as_ref();
    // A question mark consumes one character. An ASCII boundary has the
    // same width in every supported locale.
    if pattern == "?" {
        let byte = if prefix {
            value.as_bytes().first()
        } else {
            value.as_bytes().last()
        };
        if byte.is_some_and(|b| b.is_ascii() && *b != 0) {
            return Some(if prefix { 1 } else { value.len() - 1 });
        }
    }
    // A star at the end being removed can absorb everything up to a literal
    // delimiter. The nearest or farthest occurrence gives the exact cut.
    let star = if prefix {
        pattern.strip_prefix('*')
    } else {
        pattern.strip_suffix('*')
    };
    let literal = star.unwrap_or(pattern);
    let plain = group.is_none()
        && !literal
            .bytes()
            .any(|c| matches!(c, b'*' | b'?' | b'[' | b'\\' | 0))
        && memchr::memchr(0, value.as_bytes()).is_none();
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
    if group.is_none()
        && let Some((head, tail)) = pattern.split_once('*')
        && !head.is_empty()
        && !tail.is_empty()
        && !head.contains(['?', '[', '\\', '\0'])
        && !tail.contains(['*', '?', '[', '\\', '\0'])
        && memchr::memchr(0, value.as_bytes()).is_none()
    {
        if prefix {
            let rest = value.strip_prefix(head)?;
            let at = if longest {
                rest.rfind(tail)
            } else {
                rest.find(tail)
            }?;
            return Some(head.len() + at + tail.len());
        }
        let rest = value.strip_suffix(tail)?;
        return if longest {
            rest.find(head)
        } else {
            rest.rfind(head)
        };
    }
    // Try only UTF-8 boundaries, including the empty prefix or suffix.
    let mut cuts = value
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(value.len()));
    if let Some(ends) = extended::spans::match_ends(compiled, value, !prefix, longest) {
        if ends.is_empty() {
            return None;
        }
        let miss = extended::spans::MISS;
        return if prefix {
            (ends[0] != miss).then_some(ends[0])
        } else if longest {
            ends.iter().position(|&end| end != miss)
        } else {
            ends.iter().rposition(|&end| end != miss)
        };
    }
    if prefix && let Some(end) = group.and_then(|g| g.run_prefix(value, longest)) {
        return end;
    }
    let basic = group.is_none().then(|| compiled.basic());
    if let Some(matcher) = &basic
        && let Some(fixed) = compiled.width()
    {
        let cut = if prefix {
            fixed.width
        } else {
            value.len().saturating_sub(fixed.width)
        };
        let candidate = if prefix {
            value.get(..cut)
        } else {
            value.get(cut..)
        };
        if let Some(candidate) = candidate.filter(|s| s.is_ascii()) {
            return fixed
                .matches_ascii(pattern, candidate, matcher)
                .then_some(cut);
        }
    }
    let matches_at = |&i: &usize| {
        let candidate = if prefix { &value[..i] } else { &value[i..] };
        group.as_ref().map_or_else(
            || basic.as_ref().unwrap().matches(candidate),
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
    pub nocaseglob: bool,
    pub extglob: bool,
    pub globskipdots: bool,
    ignore: Option<std::rc::Rc<[IgnorePattern]>>,
}

struct IgnorePattern {
    text: String,
    compiled: std::cell::RefCell<Option<IgnoreMatcher>>,
}

struct IgnoreMatcher {
    locale: Option<std::rc::Rc<PatternLocale>>,
    extglob: bool,
    nocase: bool,
    ranges: bool,
    pattern: extended::Pattern,
}

impl From<&str> for IgnorePattern {
    fn from(text: &str) -> Self {
        Self {
            text: text.into(),
            compiled: Default::default(),
        }
    }
}

impl Default for GlobOptions {
    fn default() -> Self {
        Self {
            dotglob: false,
            nullglob: false,
            failglob: false,
            globstar: false,
            nocaseglob: false,
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
                '(' if self.extglob => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ':' if depth == 0 => {
                    if begin != i {
                        patterns.push(IgnorePattern::from(&value[begin..i]));
                    }
                    begin = i + 1;
                }
                _ => {}
            }
        }
        if begin != value.len() {
            patterns.push(IgnorePattern::from(&value[begin..]));
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
    glob_locale::run(lookup, |order, locale, cached| {
        paths(
            pattern,
            options,
            &mut names,
            order,
            locale,
            cached,
            lookup("GLOBSORT").as_deref(),
        )
    });
    names
}

pub(crate) fn unescape(component: &str) -> std::borrow::Cow<'_, str> {
    if !component.as_bytes().contains(&b'\\') {
        return std::borrow::Cow::Borrowed(component);
    }
    let mut literal = String::with_capacity(component.len());
    let mut chars = component.chars();
    while let Some(c) = chars.next() {
        literal.push(if c == '\\' {
            chars.next().unwrap_or(c)
        } else {
            c
        });
    }
    std::borrow::Cow::Owned(literal)
}

// Walk by component so POSIX character classes have the same fnmatch
// semantics in argv and in case patterns, including on Darwin.
fn paths(
    pattern: &str,
    options: &GlobOptions,
    fields: &mut Vec<String>,
    collation: Collation,
    locale: CharacterLocale,
    cached: Option<&std::rc::Rc<PatternLocale>>,
    sorting: Option<&str>,
) {
    // Linux pathname order is the UTF-8 byte order even when matching uses a named locale.
    let collation = if cfg!(target_os = "linux") {
        Collation::Bytes
    } else {
        collation
    };
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
            let mut pending = Vec::new();
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
                        &mut pending,
                    );
                }
            }
            expanded = true;
            // Adjacent globstars were collapsed above. Separated globstars may
            // reach the same pathname through different matching components.
            // Apply the requested ordering once, after the last component.
            paths = found;
            known_existing = rest.is_empty();
            continue;
        }
        let grouped = options.extglob && extended::has_group(part);
        if !grouped && !has_magic(part, true) {
            let literal = unescape(part);
            for prefix in &mut paths {
                prefix.push_str(&literal);
            }
            continue;
        }
        let extended = (grouped || options.nocaseglob)
            .then(|| extended::Pattern::for_path(part, grouped, options.nocaseglob, locale));
        let matcher = extended.is_none().then(|| Matcher::compile(part, true));
        let literal_prefix = part
            .split(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '_' | '.' | '-'))
            .next()
            .unwrap_or("");
        let folded_suffix = options
            .nocaseglob
            .then(|| part.strip_prefix(literal_prefix)?.strip_prefix('*'))
            .flatten()
            .filter(|s| {
                s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    && locale != CharacterLocale::Turkic
            });
        // A final dot followed only by ASCII letters or digits is literal,
        // even if earlier characters require fnmatch. Check it once per name.
        let required_extension = if matches!(matcher, Some(Matcher::Native(_))) {
            part.rfind('.')
                .map(|i| &part.as_bytes()[i..])
                .filter(|suffix| {
                    suffix.len() > 1 && suffix[1..].iter().all(u8::is_ascii_alphanumeric)
                })
        } else {
            None
        };
        let mut matched = Vec::new();
        for prefix in paths {
            let dir = if prefix.is_empty() { "." } else { &prefix };
            visit_names(
                dir,
                !rest.is_empty(),
                options.globskipdots || options.ignore.is_some(),
                if options.nocaseglob {
                    directory::Prefix::Folded(literal_prefix.as_bytes())
                } else {
                    directory::Prefix::Exact(literal_prefix.as_bytes())
                },
                |raw, _| {
                    if required_extension.is_some_and(|suffix| !raw.to_bytes().ends_with(suffix)) {
                        return;
                    }
                    let period = !options.dotglob || matches!(raw.to_bytes(), b"." | b"..");
                    let matches = match &extended {
                        Some(_) if folded_suffix.is_some() && raw.to_bytes().is_ascii() => {
                            let bytes = raw.to_bytes();
                            let suffix = folded_suffix.unwrap().as_bytes();
                            (!period || !bytes.starts_with(b".") || part.starts_with('.'))
                                && bytes.len() >= literal_prefix.len() + suffix.len()
                                && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
                        }
                        Some(group) => raw
                            .to_str()
                            .is_ok_and(|s| group.matches_in_locale::<false>(s, period)),
                        None => {
                            matcher
                                .as_ref()
                                .unwrap()
                                .accepts(raw.to_bytes(), period, Some(raw))
                        }
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
        paths.retain(|path| !matches!(path.trim_end_matches('/').rsplit('/').next(), Some("." | "..")));
        for pattern in patterns.iter() {
            // GLOBIGNORE tests the entire displayed pathname. An extended
            // alternative may contain a literal directory separator.
            let mut slot = pattern.compiled.borrow_mut();
            let ranges = glob_locale::collated_ranges();
            if !slot.as_ref().is_some_and(|m| {
                m.extglob == options.extglob
                    && m.nocase == options.nocaseglob
                    && m.ranges == ranges
                    && m.locale.as_ref().map(std::rc::Rc::as_ptr) == cached.map(std::rc::Rc::as_ptr)
            }) {
                *slot = Some(IgnoreMatcher {
                    locale: cached.cloned(),
                    extglob: options.extglob,
                    nocase: options.nocaseglob,
                    ranges,
                    pattern: extended::Pattern::for_path(
                        &pattern.text,
                        options.extglob,
                        options.nocaseglob,
                        locale,
                    ),
                });
            }
            let matcher = &slot.as_ref().unwrap().pattern;
            paths.retain(|path| !matcher.matches_in_locale::<true>(path, false));
        }
    }
    order::append_expansion(paths, fields, collation, sorting);
}

// The recursive component consumes zero or more directories. A symlink to a
// directory can appear in a terminal **/ listing, but is never traversed here.
fn descendants(
    prefix: String,
    terminal: bool,
    trailing: bool,
    dotglob: bool,
    found: &mut Vec<String>,
    pending: &mut Vec<String>,
) {
    let prefixed = !prefix.is_empty();
    pending.push(prefix);
    while let Some(dir) = pending.pop() {
        let base = if dir.is_empty() { "." } else { &dir };
        visit_names(
            base,
            !terminal || trailing,
            true,
            directory::Prefix::Exact(b""),
            |raw, kind| {
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
                } else if (terminal || prefixed) && is_link && std::path::Path::new(&path).is_dir()
                {
                    path.push('/');
                    found.push(path);
                }
            },
        );
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
            glob_locale::run(lookup, |order, locale, cached| {
                paths(
                    &pattern,
                    options,
                    fields,
                    order,
                    locale,
                    cached,
                    lookup("GLOBSORT").as_deref(),
                )
            });
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
            "5.txt",
            "6雪.txt",
            ".5.txt",
            "p.txt",
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
                "[56]*.txt",
                "*.[ab]",
                ".[56]*.txt",
                "a[ab]a",
                ".??*",
            ] {
                let matchers = [Matcher::new(source), Matcher::compile(source, true)];
                let native = CString::new(source).unwrap();
                for value in values {
                    let name = CString::new(value).unwrap();
                    for period in [false, true] {
                        let flags = if period { libc::FNM_PERIOD } else { 0 };
                        let expected =
                            unsafe { libc::fnmatch(native.as_ptr(), name.as_ptr(), flags) == 0 };
                        for matcher in &matchers {
                            assert_eq!(
                                matcher.accepts(value.as_bytes(), period, None),
                                expected,
                                "{source:?} {value:?} {flags}"
                            );
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn trim_boundaries_agree_with_glob_matching() {
        for text in ["", "aaaa", "/a/雪/雪", ".log", "a.log.log", "a*b"] {
            for pattern in [
                "",
                "*",
                ".log",
                "*aa",
                "aa*",
                "*/",
                "/*",
                "*雪",
                "雪*",
                "a*z",
                "*?",
                "[a]*",
                r"\*",
                "a\\*b",
                "??",
                "??-??",
                "[a-z]?",
                "[[:alpha:]]?",
                "[[=a=]]?",
                r"\??",
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
                            trim_boundary(
                                text,
                                &ParameterPattern::new(pattern, false, None),
                                prefix,
                                longest
                            ),
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
