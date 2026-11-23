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
    FoldedClass(bracket::FoldedClass),
    Any,
    Star,
    Class(Matcher<Box<str>>, std::cell::Cell<(u128, u128)>),
    Choice(Vec<Box<str>>),
    Group(u8, Vec<Vec<Atom>>),
}

fn sequence(source: &mut &str, grouped: bool, extended: bool) -> Option<Vec<Atom>> {
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
                alternatives.push(sequence(source, true, extended)?);
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
        let (atom, count) = match bytes[0] {
            b'?' => (Atom::Any, 1),
            b'*' => (Atom::Star, 1),
            b'[' if let Some(end) = bracket::class_end(&bytes[1..]) => {
                let len = end + 2;
                (
                    Atom::Class(Matcher::new(rest[..len].into()), Default::default()),
                    len,
                )
            }
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

fn fold_atoms(atoms: &mut [Atom], locale: super::CharacterLocale) {
    for atom in atoms {
        match atom {
            Atom::Text(text) => *atom = Atom::FoldedText(std::mem::take(text), locale),
            Atom::Choice(choices) => {
                let alternatives = std::mem::take(choices)
                    .into_iter()
                    .map(|s| vec![Atom::FoldedText(s, locale)])
                    .collect();
                *atom = Atom::Group(b'@', alternatives);
            }
            Atom::Group(_, alternatives) => {
                for branch in alternatives {
                    fold_atoms(branch, locale);
                }
            }
            Atom::Class(class, _) if !class.source().is_empty() => {
                *atom = Atom::FoldedClass(bracket::FoldedClass::new(class.source(), locale))
            }
            _ => {}
        }
    }
}

fn one_character<const PATHNAME: bool>(atom: &Atom, value: &str, pos: usize) -> Option<usize> {
    let c = value[pos..].chars().next()?;
    if PATHNAME && c == '/' && matches!(atom, Atom::Any) {
        return None;
    }
    let end = pos + c.len_utf8();
    if let Atom::FoldedClass(class) = atom {
        return class.accepts(c).then_some(end);
    }
    if let Atom::Class(class, cache) = atom {
        let (known, matched) = cache.get();
        let bit = if c.is_ascii() { 1u128 << (c as u32) } else { 0 };
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
                        | Atom::FoldedClass(_)
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
                Atom::Any | Atom::Class(..) | Atom::FoldedClass(_) => {
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
        Atom::Choice(options) => {
            let mut matches = options.iter().filter_map(|s| strip(s));
            let first = matches.next();
            if matches.next().is_some() {
                Err(())
            } else {
                Ok(first)
            }
        }
        _ => unreachable!(),
    }
}

#[derive(Debug)]
pub(crate) struct Pattern {
    atoms: Vec<Atom>,
    single_run: Option<usize>,
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
        fold_atoms(&mut pattern.atoms, locale);
        pattern.single_run = None;
        pattern
    }

    pub(crate) fn parse(mut text: &str, extended: bool) -> Self {
        let original = text;
        let atoms = sequence(&mut text, false, extended)
            .unwrap_or_else(|| vec![Atom::Text(original.into())]);
        let mut variable = atoms
            .iter()
            .enumerate()
            .filter(|(_, atom)| !matches!(atom, Atom::Text(_) | Atom::Choice(_)));
        let single_run = variable
            .next()
            .and_then(|(i, atom)| unit_run(atom).map(|_| i))
            .filter(|_| variable.next().is_none());
        Self { atoms, single_run }
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
        self.matches::<false>(value, period)
    }

    pub(crate) fn pathname(&self, value: &str) -> bool {
        self.matches::<true>(value, false)
    }

    fn matches<const PATHNAME: bool>(&self, value: &str, period: bool) -> bool {
        let period = period && value.starts_with('.');
        super::localized(|| {
            if let Some(i) = self.single_run
                && let Some(result) = self.match_run::<PATHNAME>(value, i, period)
            {
                return result;
            }
            endings::<PATHNAME>(&self.atoms, value, 0, period)
                .binary_search(&value.len())
                .is_ok()
        })
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
