// Bash enables extended glob operators on the pattern side of [[ == ]].
//
// Evaluate each alternative as a set of reachable character boundaries.
// Empty alternatives and nested repetition can reach the same boundary more
// than once. Each repetition visits a reachable boundary only once.
use super::{Matcher, bracket};

#[derive(Debug)]
enum Atom {
    Text(Box<str>),
    FoldedText(Box<str>, super::CharacterLocale),
    CharacterClass(bracket::CharacterClass),
    Any,
    Star,
    Class(Matcher<Box<str>>, std::cell::Cell<(u128, u128)>),
    Choice(Vec<Box<str>>),
    Group(u8, Vec<Vec<Atom>>),
}

fn sequence<const PATHNAME: bool>(
    source: &mut &str,
    grouped: bool,
    extended: bool,
) -> Option<Vec<Atom>> {
    let mut atoms = Vec::new();
    while !source.is_empty() {
        let rest = *source;
        let bytes = rest.as_bytes();
        if grouped && matches!(bytes[0], b'|' | b')') {
            break;
        }
        if extended
            && matches!(bytes[0], b'@' | b'!' | b'?' | b'*' | b'+')
            && bytes.get(1) == Some(&b'(')
        {
            *source = &rest[2..];
            let mut alternatives = Vec::new();
            loop {
                alternatives.push(sequence::<PATHNAME>(source, true, extended)?);
                let sep = source.as_bytes().first()?;
                *source = &source[1..];
                if *sep == b')' {
                    break;
                }
            }
            if bytes[0] == b'@'
                && alternatives
                    .iter()
                    .all(|seq| matches!(seq.as_slice(), [Atom::Text(_)]))
            {
                let choices = alternatives
                    .into_iter()
                    .map(|mut seq| {
                        let Atom::Text(text) = seq.pop().unwrap() else {
                            unreachable!()
                        };
                        text
                    })
                    .collect();
                atoms.push(Atom::Choice(choices));
            } else {
                atoms.push(Atom::Group(bytes[0], alternatives));
            }
            continue;
        }
        let class = &bytes[1..];
        if bytes[0] == b'['
            && let Some(end) = bracket::class_end(class)
            && !(PATHNAME && class[..=end].contains(&b'/'))
        {
            let len = end + 2;
            atoms.push(Atom::Class(
                Matcher::new(rest[..len].into()),
                Default::default(),
            ));
            *source = &rest[len..];
            continue;
        }
        let (atom, count) = match bytes[0] {
            b'?' => (Atom::Any, 1),
            b'*' => (Atom::Star, 1),
            b'[' if grouped => return None,
            b'\\' if bytes.len() > 1 => {
                let end = 1 + rest[1..].chars().next()?.len_utf8();
                (Atom::Text(rest[1..end].into()), end)
            }
            _ => {
                let end = rest
                    .char_indices()
                    .skip(1)
                    .find(|&(i, c)| {
                        matches!(c, '*' | '?' | '[' | '\\')
                            || grouped && matches!(c, ')' | '|')
                            || matches!(c, '@' | '+' | '!') && rest[i + 1..].starts_with('(')
                    })
                    .map_or(rest.len(), |(i, _)| i);
                (Atom::Text(rest[..end].into()), end)
            }
        };
        atoms.push(atom);
        *source = &rest[count..];
    }
    Some(atoms)
}

fn prepare_classes_and_case(atoms: &mut [Atom], fold: Option<super::CharacterLocale>) {
    for atom in atoms {
        match atom {
            Atom::Text(text) if let Some(locale) = fold => {
                *atom = Atom::FoldedText(std::mem::take(text), locale)
            }
            Atom::Choice(choices) if let Some(locale) = fold => {
                let alternatives = std::mem::take(choices)
                    .into_iter()
                    .map(|s| vec![Atom::FoldedText(s, locale)])
                    .collect();
                *atom = Atom::Group(b'@', alternatives);
            }
            Atom::Group(_, alternatives) => {
                for branch in alternatives {
                    prepare_classes_and_case(branch, fold);
                }
            }
            Atom::Class(class, _) if !class.source().is_empty() => {
                *atom = Atom::CharacterClass(bracket::CharacterClass::new(class.source(), fold))
            }
            _ => {}
        }
    }
}

fn one_character<const PATHNAME: bool>(atom: &Atom, value: &str, pos: usize) -> Option<usize> {
    let c = value[pos..].chars().next()?;
    if PATHNAME && c == '/' {
        return None;
    }
    let end = pos + c.len_utf8();
    if let Atom::CharacterClass(class) = atom {
        return class.accepts(c).then_some(end);
    }
    if let Atom::Class(class, cache) = atom {
        let (known, matched) = cache.get();
        let bit = if c.is_ascii() && !super::glob_locale::collated_ranges() {
            1u128 << (c as u32)
        } else {
            0
        };
        if known & bit != 0 {
            return (matched & bit != 0).then_some(end);
        }
        let mut buffer = [0u8; 5];
        let bytes = c.encode_utf8(&mut buffer[..4]).as_bytes();
        let length = bytes.len();
        let name = std::ffi::CStr::from_bytes_with_nul(&buffer[..=length]).ok()?;
        let accepted = class.accepts(name.to_bytes(), false, Some(name));
        if bit != 0 {
            cache.set((known | bit, if accepted { matched | bit } else { matched }));
        }
        if !accepted {
            return None;
        }
    }
    Some(end)
}

fn endings<const PATHNAME: bool>(
    atoms: &[Atom],
    value: &str,
    start: usize,
    period: bool,
) -> Vec<usize> {
    let mut points = vec![start];
    let mut matched_end = false;
    let mut next = Vec::new();
    for (i, atom) in atoms.iter().enumerate() {
        next.clear();
        for &pos in &points {
            if period
                && pos == 0
                && matches!(
                    atom,
                    Atom::Any
                        | Atom::Class(..)
                        | Atom::CharacterClass(_)
                        | Atom::Star
                        | Atom::Group(b'!', _)
                )
            {
                continue;
            }
            match atom {
                Atom::Text(text) if value[pos..].starts_with(text.as_ref()) => {
                    next.push(pos + text.len())
                }
                Atom::Text(_) => {}
                Atom::FoldedText(text, locale) => {
                    if let Some(length) = super::folded_prefix(&value[pos..], text, *locale) {
                        next.push(pos + length);
                    }
                }
                Atom::Choice(choices) => {
                    for text in choices {
                        if value[pos..].starts_with(text.as_ref()) {
                            next.push(pos + text.len());
                        }
                    }
                }
                Atom::Any | Atom::Class(..) | Atom::CharacterClass(_) => {
                    if let Some(end) = one_character::<PATHNAME>(atom, value, pos) {
                        next.push(end);
                    }
                }
                Atom::Star => {
                    if PATHNAME
                        && atoms.get(i + 1).is_some()
                        && let Some(slash) = value[pos..].find('/')
                    {
                        next.extend(value[pos..pos + slash].char_indices().map(|(i, _)| pos + i));
                        next.push(pos + slash);
                        continue;
                    }
                    next.extend(value[pos..].char_indices().map(|(i, _)| pos + i));
                    // A plain star searches for the start of these groups
                    // only at existing characters. GNU Bash gives a trailing
                    // empty input its own result for a following negation.
                    match atoms.get(i + 1) {
                        Some(Atom::Group(b'!' | b'+' | b'@', _) | Atom::Choice(_)) => {
                            matched_end |= matches!(atoms.get(i + 1), Some(Atom::Group(b'!', _)))
                                && points.last() == Some(&value.len());
                        }
                        _ => next.push(value.len()),
                    }
                    // points is sorted, so all later starting positions are covered.
                    break;
                }
                Atom::Group(op, alternatives) => {
                    if let [sequence] = alternatives.as_slice()
                        && let [unit @ (Atom::Any | Atom::Class(..))] = sequence.as_slice()
                        && matches!(op, b'+' | b'*')
                    {
                        if *op == b'*' {
                            next.push(pos);
                        }
                        if !period || pos != 0 {
                            let mut previous = pos;
                            while let Some(end) = one_character::<PATHNAME>(unit, value, previous) {
                                next.push(end);
                                previous = end;
                            }
                        }
                        continue;
                    }
                    let options = |from| {
                        alternatives
                            .iter()
                            .flat_map(move |seq| endings::<PATHNAME>(seq, value, from, period))
                    };
                    if *op == b'!' {
                        let mut excluded = options(pos).collect::<Vec<_>>();
                        excluded.sort_unstable();
                        next.extend(
                            value[pos..]
                                .char_indices()
                                .map(|(i, _)| pos + i)
                                .chain(std::iter::once(value.len()))
                                .filter(|i| excluded.binary_search(i).is_err()),
                        );
                    } else {
                        if matches!(op, b'?' | b'*') {
                            next.push(pos);
                        }
                        if matches!(op, b'+' | b'*') {
                            let mut seen = std::collections::BTreeSet::new();
                            let mut pending = vec![pos];
                            while let Some(p) = pending.pop() {
                                for end in options(p) {
                                    if seen.insert(end) && end != p {
                                        pending.push(end);
                                    }
                                }
                            }
                            next.extend(seen);
                        } else {
                            next.extend(options(pos));
                        }
                    }
                }
            }
        }
        if next.is_empty() {
            return if matched_end { vec![value.len()] } else { next };
        }
        if next.len() > 1 {
            next.sort_unstable();
            next.dedup();
        }
        std::mem::swap(&mut points, &mut next);
    }
    if matched_end && points.last() != Some(&value.len()) {
        points.push(value.len());
    }
    points
}

// If there is only one variable-width run, the literals on either side can
// determine its boundaries without constructing a list of alternatives.
fn unit_run(atom: &Atom) -> Option<(&Atom, bool)> {
    let Atom::Group(op @ (b'+' | b'*'), alternatives) = atom else {
        return None;
    };
    let [sequence] = alternatives.as_slice() else {
        return None;
    };
    let [unit @ (Atom::Any | Atom::Class(..))] = sequence.as_slice() else {
        return None;
    };
    Some((unit, *op == b'*'))
}

fn literal_edge<'v>(atom: &Atom, value: &'v str, prefix: bool) -> Result<Option<&'v str>, ()> {
    let strip = |text: &str| {
        if prefix {
            value.strip_prefix(text)
        } else {
            value.strip_suffix(text)
        }
    };
    match atom {
        Atom::Text(text) => Ok(strip(text)),
        Atom::FoldedText(text, locale) => {
            let edge = if prefix {
                value.get(..text.len())
            } else {
                value
                    .len()
                    .checked_sub(text.len())
                    .and_then(|n| value.get(n..))
            };
            if !matches!(locale, super::CharacterLocale::Turkic)
                && let Some(edge) = edge
                && edge.is_ascii()
                && text.is_ascii()
            {
                return Ok(edge.eq_ignore_ascii_case(text).then(|| {
                    if prefix {
                        &value[text.len()..]
                    } else {
                        &value[..value.len() - text.len()]
                    }
                }));
            }
            if prefix {
                return Ok(super::folded_prefix(value, text, *locale).map(|n| &value[n..]));
            }
            let mut chars = value.chars();
            for c in text.chars().rev() {
                if !chars
                    .next_back()
                    .is_some_and(|v| super::same_letter(v, c, *locale))
                {
                    return Ok(None);
                }
            }
            Ok(Some(chars.as_str()))
        }
        Atom::Choice(options) => {
            let mut matches = options.iter().filter_map(|s| strip(s));
            let first = matches.next();
            if matches.next().is_some() {
                Err(())
            } else {
                Ok(first)
            }
        }
        _ => Err(()),
    }
}

#[derive(Debug)]
pub(crate) struct Pattern {
    atoms: Vec<Atom>,
    single_run: Option<usize>,
    bytewise_path: bool,
}

fn pathname_units(text: &str, bytewise: bool) -> std::borrow::Cow<'_, str> {
    if bytewise && !text.is_ascii() {
        std::borrow::Cow::Owned(text.bytes().map(char::from).collect())
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

impl Pattern {
    pub(crate) fn new(text: &str) -> Option<Self> {
        if !has_group(text) {
            return None;
        }
        Some(Self::parse(text, true))
    }

    pub(crate) fn insensitive(text: &str, extended: bool, locale: super::CharacterLocale) -> Self {
        let mut pattern = Self::parse(text, extended);
        prepare_classes_and_case(&mut pattern.atoms, Some(locale));
        pattern.single_run = None;
        pattern
    }

    pub(super) fn for_path(
        text: &str,
        extended: bool,
        nocase: bool,
        locale: super::CharacterLocale,
    ) -> Self {
        // In the C locale each byte is a matching unit for ?, brackets and
        // extended patterns. Literal edges around a star can use the original UTF-8.
        let bytewise = matches!(locale, super::CharacterLocale::Ascii)
            && (text.contains(['?', '[']) || extended && has_group(text));
        let text = pathname_units(text, bytewise);
        let mut pattern = Self::parse_mode::<true>(&text, extended);
        if nocase || bytewise {
            prepare_classes_and_case(&mut pattern.atoms, nocase.then_some(locale));
            pattern.single_run = None;
        }
        pattern.bytewise_path = bytewise;
        pattern
    }

    pub(crate) fn parse(text: &str, extended: bool) -> Self {
        Self::parse_mode::<false>(text, extended)
    }

    fn parse_mode<const PATHNAME: bool>(mut text: &str, extended: bool) -> Self {
        let original = text;
        let atoms = sequence::<PATHNAME>(&mut text, false, extended)
            .unwrap_or_else(|| vec![Atom::Text(original.into())]);
        let mut variable = atoms
            .iter()
            .enumerate()
            .filter(|(_, atom)| !matches!(atom, Atom::Text(_) | Atom::Choice(_)));
        let single_run = variable
            .next()
            .and_then(|(i, atom)| unit_run(atom).map(|_| i))
            .filter(|_| variable.next().is_none());
        Self {
            atoms,
            single_run,
            bytewise_path: false,
        }
    }

    fn match_run<const PATHNAME: bool>(&self, value: &str, i: usize, period: bool) -> Option<bool> {
        let mut rest = value;
        for atom in &self.atoms[..i] {
            let Some(next) = literal_edge(atom, rest, true).ok()? else {
                return Some(false);
            };
            rest = next;
        }
        let at_start = rest.len() == value.len();
        for atom in self.atoms[i + 1..].iter().rev() {
            let Some(next) = literal_edge(atom, rest, false).ok()? else {
                return Some(false);
            };
            rest = next;
        }
        let (unit, empty) = unit_run(&self.atoms[i])?;
        if rest.is_empty() {
            return Some(empty);
        }
        if at_start && period {
            return Some(false);
        }
        let mut pos = 0;
        while pos < rest.len() {
            let Some(end) = one_character::<PATHNAME>(unit, rest, pos) else {
                return Some(false);
            };
            pos = end;
        }
        Some(true)
    }

    pub(crate) fn accepts(&self, value: &str, period: bool) -> bool {
        super::localized(|| self.matches_in_locale::<false>(value, period))
    }

    // The star consumes the gap between literal edges. Bash also lets a final
    // star consume slashes when it tests the whole pathname against GLOBIGNORE.
    fn match_star<const PATHNAME: bool>(&self, value: &str, period: bool) -> Option<bool> {
        use Atom::Star;
        let (head, tail) = match self.atoms.as_slice() {
            [Star] => return Some(!period),
            [head, Star] => (Some(head), None),
            [Star, tail] => (None, Some(tail)),
            [head, Star, tail] => (Some(head), Some(tail)),
            _ => return None,
        };
        let mut gap = value;
        if let Some(head) = head {
            let Some(rest) = literal_edge(head, gap, true).ok()? else {
                return Some(false);
            };
            gap = rest;
        }
        if period && gap.len() == value.len() {
            return Some(false);
        }
        if let Some(tail) = tail {
            let Some(rest) = literal_edge(tail, gap, false).ok()? else {
                return Some(false);
            };
            return Some(!PATHNAME || !rest.contains('/'));
        }
        Some(true)
    }

    pub(super) fn matches_in_locale<const PATHNAME: bool>(
        &self,
        value: &str,
        period: bool,
    ) -> bool {
        let units = pathname_units(value, self.bytewise_path);
        let value = units.as_ref();
        let period = period && value.starts_with('.');
        if let Some(result) = self.match_star::<PATHNAME>(value, period) {
            return result;
        }
        if let Some(i) = self.single_run
            && let Some(result) = self.match_run::<PATHNAME>(value, i, period)
        {
            return result;
        }
        let mut atoms = self.atoms.as_slice();
        let mut pos = 0;
        while let [Atom::Text(text), remaining @ ..] = atoms {
            if !value[pos..].starts_with(text.as_ref()) {
                return false;
            }
            pos += text.len();
            atoms = remaining;
        }
        if atoms.is_empty() {
            return pos == value.len();
        }
        endings::<PATHNAME>(atoms, value, pos, period)
            .binary_search(&value.len())
            .is_ok()
    }
}

pub(crate) fn has_group(text: &str) -> bool {
    if !text.contains('(') {
        return false;
    }
    text.as_bytes()
        .windows(2)
        .any(|pair| matches!(pair, [b'@' | b'!' | b'?' | b'*' | b'+', b'(']))
}

pub(crate) fn matches(value: &str, pattern: &str) -> bool {
    Pattern::new(pattern).map_or_else(
        || super::matches(value, pattern),
        |p| p.accepts(value, false),
    )
}

pub(crate) fn insensitive_match(
    value: &str,
    pattern: &str,
    extended: bool,
    locale: super::CharacterLocale,
) -> bool {
    if !pattern
        .bytes()
        .any(|b| matches!(b, b'?' | b'[' | b'\\' | 0))
        && !(extended && has_group(pattern))
    {
        match pattern.split_once('*') {
            None => return super::folded_prefix(value, pattern, locale) == Some(value.len()),
            Some((head, tail)) if !tail.contains('*') => {
                let Some(offset) = super::folded_prefix(value, head, locale) else {
                    return false;
                };
                let mut rest = value[offset..].chars().rev();
                return tail.chars().rev().all(|c| {
                    rest.next()
                        .is_some_and(|v| super::same_letter(v, c, locale))
                });
            }
            _ => {}
        }
    }
    Pattern::insensitive(pattern, extended, locale).accepts(value, false)
}
