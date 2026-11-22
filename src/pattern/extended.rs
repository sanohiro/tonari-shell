// Bash enables extended glob operators on the pattern side of [[ == ]].
//
// Evaluate each alternative as a set of reachable character boundaries.
// Empty alternatives and nested repetition can reach the same boundary more
// than once. Each repetition visits a reachable boundary only once.
use super::{Matcher, bracket};

#[derive(Debug)]
enum Atom<'a> {
    Text(&'a str),
    Any,
    Star,
    Class(Matcher<&'a str>),
    Group(u8, Vec<Vec<Atom<'a>>>),
}

fn sequence<'a>(source: &mut &'a str, grouped: bool) -> Option<Vec<Atom<'a>>> {
    let mut atoms = Vec::new();
    while !source.is_empty() {
        let rest = *source;
        let bytes = rest.as_bytes();
        if grouped && matches!(bytes[0], b'|' | b')') {
            break;
        }
        if matches!(bytes[0], b'@' | b'!' | b'?' | b'*' | b'+') && bytes.get(1) == Some(&b'(') {
            *source = &rest[2..];
            let mut alternatives = Vec::new();
            loop {
                alternatives.push(sequence(source, true)?);
                let sep = source.as_bytes().first()?;
                *source = &source[1..];
                if *sep == b')' {
                    break;
                }
            }
            atoms.push(Atom::Group(bytes[0], alternatives));
            continue;
        }
        let (atom, count) = match bytes[0] {
            b'?' => (Atom::Any, 1),
            b'*' => (Atom::Star, 1),
            b'[' if let Some(end) = bracket::class_end(&bytes[1..]) => {
                let len = end + 2;
                (Atom::Class(Matcher::new(&rest[..len])), len)
            }
            b'\\' if bytes.len() > 1 => {
                let end = 1 + rest[1..].chars().next()?.len_utf8();
                (Atom::Text(&rest[1..end]), end)
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
                (Atom::Text(&rest[..end]), end)
            }
        };
        atoms.push(atom);
        *source = &rest[count..];
    }
    Some(atoms)
}

fn endings(atoms: &[Atom<'_>], value: &str, start: usize, period: bool) -> Vec<usize> {
    let mut points = vec![start];
    let mut matched_end = false;
    for (i, atom) in atoms.iter().enumerate() {
        let mut next = Vec::new();
        for &pos in &points {
            if period
                && pos == 0
                && matches!(
                    atom,
                    Atom::Any | Atom::Class(_) | Atom::Star | Atom::Group(b'!', _)
                )
            {
                continue;
            }
            match atom {
                Atom::Text(text) if value[pos..].starts_with(text) => next.push(pos + text.len()),
                Atom::Text(_) => {}
                Atom::Any | Atom::Class(_) => {
                    if let Some(c) = value[pos..].chars().next() {
                        let end = pos + c.len_utf8();
                        if matches!(atom, Atom::Any)
                            || matches!(atom, Atom::Class(m) if m.matches(&value[pos..end]))
                        {
                            next.push(end);
                        }
                    }
                }
                Atom::Star => {
                    next.extend(value[pos..].char_indices().map(|(i, _)| pos + i));
                    // A plain star searches for the start of these groups
                    // only at existing characters. GNU Bash gives a trailing
                    // empty input its own result for a following negation.
                    match atoms.get(i + 1) {
                        Some(Atom::Group(b'!' | b'+' | b'@', _)) => {
                            matched_end |= matches!(atoms.get(i + 1), Some(Atom::Group(b'!', _)))
                                && points.last() == Some(&value.len());
                        }
                        _ => next.push(value.len()),
                    }
                    // points is sorted, so all later starting positions are covered.
                    break;
                }
                Atom::Group(op, alternatives) => {
                    let options = |from| {
                        alternatives
                            .iter()
                            .flat_map(move |seq| endings(seq, value, from, period))
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
        next.sort_unstable();
        next.dedup();
        points = next;
    }
    if matched_end && points.last() != Some(&value.len()) {
        points.push(value.len());
    }
    points
}

pub(crate) struct Pattern<'a>(Vec<Atom<'a>>);

impl<'a> Pattern<'a> {
    pub(crate) fn new(mut text: &'a str) -> Option<Self> {
        if !has_group(text) {
            return None;
        }
        sequence(&mut text, false).map(Self)
    }

    pub(crate) fn accepts(&self, value: &str, period: bool) -> bool {
        endings(&self.0, value, 0, period && value.starts_with('.'))
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
