// Bash enables extended glob operators on the pattern side of [[ == ]].
//
// Evaluate each alternative as a set of reachable character boundaries.
// Empty alternatives and nested repetition can reach the same boundary more
// than once. Each repetition visits a reachable boundary only once.
pub(crate) mod spans;

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
    depth: usize,
    extended: bool,
) -> Option<Vec<Atom>> {
    let mut atoms = Vec::new();
    let last_close = std::cell::OnceCell::new();
    while !source.is_empty() {
        let rest = *source;
        let bytes = rest.as_bytes();
        if depth != 0 && matches!(bytes[0], b'|' | b')') {
            break;
        }
        if extended
            && matches!(bytes[0], b'@' | b'!' | b'?' | b'*' | b'+')
            && bytes.get(1) == Some(&b'(')
        {
            *source = &rest[2..];
            if depth >= 64
                || depth >= 8 && depth.is_multiple_of(8) && crate::stack_budget_exceeded()
            {
                return None;
            }
            let mut alternatives = Vec::new();
            let closed = loop {
                let Some(sequence) = sequence::<PATHNAME>(source, depth + 1, extended) else { break false; };
                alternatives.push(sequence);
                let Some(&sep) = source.as_bytes().first() else { break false; };
                *source = &source[1..];
                if sep == b')' { break true; }
            };
            if !closed {
                atoms.push(Atom::Text(rest.into()));
                *source = "";
                break;
            }
            if !PATHNAME && matches!(bytes[0],b'*'|b'+'|b'@')
                && let [only] = alternatives.as_mut_slice()
                && let [Atom::Group(inner, _)] = only.as_slice()
                && (bytes[0]==b'@' || matches!(*inner,b'*'|b'+')) {
                let Atom::Group(inner,options)=only.pop().unwrap() else { unreachable!() };
                let outer=bytes[0];
                let kind=if outer==b'@' {inner} else if inner==b'*'||outer==b'*' {b'*'} else {b'+'};
                atoms.push(Atom::Group(kind,options));
                continue;
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
            && last_close.get_or_init(|| memchr::memrchr(b']',bytes).map(|i|bytes.len()-i)).is_some_and(|distance|distance <= class.len())
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
            b'[' if depth != 0 => return None,
            // An unfinished escape after a wildcard has no matching alternative.
            b'\\' if bytes.len() == 1 && matches!(atoms.last(), Some(Atom::Star)) => {
                (Atom::Choice(Vec::new()), 1)
            }
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
                            || depth != 0 && matches!(c, ')' | '|')
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

// Repetition from several starting offsets is one reachability problem. A
// boundary can be both an original start and the result of a nonempty path.
fn repeat_ends<const PATHNAME: bool>(
    options: &[Vec<Atom>],
    value: &str,
    starts: &[usize],
    period: bool,
    empty: bool,
    out: &mut Vec<usize>,
) {
    let mut visited = vec![0u8; value.len() - starts[0] + 1];
    let mut pending = starts.to_vec();
    let first = starts[0];
    for &i in starts {
        visited[i - first] = if empty { 3 } else { 1 };
    }
    while let Some(i) = pending.pop() {
        let mut reach = |end: usize| {
            let entry = &mut visited[end - first];
            if *entry == 0 {
                pending.push(end);
            }
            *entry = 3;
        };
        for branch in options {
            match branch.as_slice() {
                [] => reach(i),
                [Atom::Text(text)] => {
                    if value[i..].starts_with(&**text) {
                        reach(i + text.len());
                    }
                }
                [Atom::FoldedText(text, locale)] => {
                    if let Some(n) = super::folded_prefix(&value[i..], text, *locale) {
                        reach(i + n);
                    }
                }
                [Atom::Choice(words)] => {
                    for text in words {
                        if value[i..].starts_with(&**text) {
                            reach(i + text.len());
                        }
                    }
                }
                [a @ (Atom::Any | Atom::Class(..) | Atom::CharacterClass(_))] => {
                    if (!period || i != 0)
                        && let Some(end) = one_character::<PATHNAME>(a, value, i)
                    {
                        reach(end);
                    }
                }
                _ => endings::<PATHNAME>(branch, value, i, period)
                    .into_iter()
                    .for_each(&mut reach),
            }
        }
    }
    out.extend(
        visited
            .into_iter()
            .enumerate()
            .filter_map(|(i, state)| (state == 3).then_some(first + i)),
    );
}

fn endings<const PATHNAME: bool>(
    atoms: &[Atom],
    value: &str,
    start: usize,
    period: bool,
) -> Vec<usize> {
    let mut points = vec![start];
    let mut next = Vec::new();
    for (i, atom) in atoms.iter().enumerate() {
        next.clear();
        let mut holes: Option<Vec<usize>> = None;
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
                            if matches!(atoms.get(i + 1), Some(Atom::Group(b'!', _)))
                                && points.last() == Some(&value.len())
                            {
                                next.push(value.len());
                            }
                        }
                        _ => next.push(value.len()),
                    }
                    // points is sorted, so all later starting positions are covered.
                    break;
                }
                Atom::Group(op, alternatives) => {
                    if let [sequence] = alternatives.as_slice()
                        && let [unit @ (Atom::Any | Atom::Class(..) | Atom::CharacterClass(_))] =
                            sequence.as_slice()
                        && points.len() == 1
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
                        // The first start reaches every boundary except its
                        // excluded ends. Later starts need only fill those holes.
                        if holes
                            .as_ref()
                            .is_some_and(|v| v.last().is_none_or(|&end| end < pos))
                        {
                            break;
                        }
                        let mut excluded = options(pos).collect::<Vec<_>>();
                        excluded.sort_unstable();
                        if let Some(missing) = &mut holes {
                            missing.retain(|&end| {
                                let keep = end < pos || excluded.binary_search(&end).is_ok();
                                if !keep {
                                    next.push(end);
                                }
                                keep
                            });
                        } else {
                            next.extend(
                                value[pos..]
                                    .char_indices()
                                    .map(|(i, _)| pos + i)
                                    .chain(std::iter::once(value.len()))
                                    .filter(|i| excluded.binary_search(i).is_err()),
                            );
                            excluded.dedup();
                            holes = Some(excluded);
                        }
                    } else {
                        if matches!(op, b'+' | b'*') {
                            repeat_ends::<PATHNAME>(
                                alternatives,
                                value,
                                &points,
                                period,
                                *op == b'*',
                                &mut next,
                            );
                            break;
                        }
                        if *op == b'?' {
                            next.push(pos);
                        }
                        next.extend(options(pos));
                    }
                }
            }
        }
        if next.is_empty() {
            return next;
        }
        if next.len() > 1 {
            next.sort_unstable();
            next.dedup();
        }
        std::mem::swap(&mut points, &mut next);
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
            Ok(super::case::strip_folded_suffix(value, text, *locale))
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
    independent_prefix: bool,
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
        if extended && text.starts_with("@(@(") {
            let wrappers = text.as_bytes().chunks_exact(2).take_while(|s| *s == b"@(").count();
            let closing = text.as_bytes().iter().rev().take_while(|&&b| b == b')').count();
            if closing == wrappers && 3*wrappers <= text.len()
                && !text[2*wrappers..text.len()-wrappers].contains(['(',')','\\']) {
                // One outermost @ group suffices even when the inner text has alternatives.
                text = &text[2*(wrappers-1)..text.len()-(wrappers-1)];
            }
        }
        let original = text;
        let atoms = sequence::<PATHNAME>(&mut text, 0, extended)
            .unwrap_or_else(|| vec![Atom::Text(original.into())]);
        let mut variable = atoms
            .iter()
            .enumerate()
            .filter(|(_, atom)| !matches!(atom, Atom::Text(_) | Atom::Choice(_)));
        let single_run = variable
            .next()
            .and_then(|(i, atom)| unit_run(atom).map(|_| i))
            .filter(|_| variable.next().is_none());
        fn independent(atoms: &[Atom]) -> bool {
            atoms.iter().all(|atom| match atom {
                Atom::Star | Atom::Group(b'!', _) => false,
                Atom::Group(_, alternatives) => alternatives.iter().all(|seq| independent(seq)),
                _ => true,
            })
        }
        let independent_prefix = !PATHNAME && independent(&atoms);
        Self {
            atoms,
            independent_prefix,
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

    // With no plain star or negation, a reachable prefix end is independent of
    // the bytes following it. Query all ends once for ambiguous alternatives.
    pub(crate) fn run_prefix(&self, value: &str, longest: bool) -> Option<Option<usize>> {
        if let Some(result) = self.repeated_prefix(value, longest) {
            return Some(result);
        }
        self.independent_prefix.then(|| {
            super::localized(|| {
                let points = endings::<false>(&self.atoms, value, 0, false);
                if longest {
                    points.last().copied()
                } else {
                    points.first().copied()
                }
            })
        })
    }

    fn repeated_prefix(&self, value: &str, longest: bool) -> Option<Option<usize>> {
        let [atom] = self.atoms.as_slice() else {
            return None;
        };
        let Atom::Group(op @ (b'+' | b'*'), alternatives) = atom else {
            return None;
        };
        let empty = *op == b'*';
        let unit = unit_run(atom).map(|(unit, _)| unit);
        // Equal-width literal alternatives cannot overlap at different end
        // offsets. A greedy run is therefore also the longest possible one.
        let literal_width = |seq: &Vec<Atom>| match seq.as_slice() {
            [Atom::Text(s)] if !s.is_empty() => Some(s.len()),
            _ => None,
        };
        if unit.is_none() {
            let width = literal_width(alternatives.first()?)?;
            if !alternatives
                .iter()
                .all(|seq| literal_width(seq) == Some(width))
            {
                return None;
            }
        }
        let advance = |pos| {
            if let Some(unit) = unit {
                one_character::<false>(unit, value, pos)
            } else {
                alternatives.iter().find_map(|seq| match &seq[0] {
                    Atom::Text(s) if value[pos..].starts_with(&**s) => Some(pos + s.len()),
                    _ => None,
                })
            }
        };
        Some(super::localized(|| {
            if !longest && empty {
                return Some(0);
            }
            let mut end = 0;
            while let Some(next) = advance(end) {
                end = next;
                if !longest {
                    break;
                }
            }
            (empty || end != 0).then_some(end)
        }))
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
        // A nullable group cannot consume the literal required after it.
        if let Some(tail @ (Atom::Text(_) | Atom::FoldedText(..))) = self.atoms.last()
            && matches!(literal_edge(tail, value, false), Ok(None))
        {
            return false;
        }
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
        while let [
            edge @ (Atom::Text(_) | Atom::FoldedText(..) | Atom::Choice(_)),
            remaining @ ..,
        ] = atoms
        {
            match literal_edge(edge, &value[pos..], true) {
                Ok(Some(rest)) => pos = value.len() - rest.len(),
                Ok(None) => return false,
                Err(()) => break, // overlapping alternatives need backtracking
            }
            atoms = remaining;
        }
        while let [
            unit @ (Atom::CharacterClass(_) | Atom::Class(..) | Atom::Any),
            rest @ ..,
        ] = atoms
        {
            if period && pos == 0 {
                return false;
            }
            let Some(end) = one_character::<PATHNAME>(unit, value, pos) else {
                return false;
            };
            pos = end;
            atoms = rest;
        }
        match atoms {
            [Atom::Star] => return !period || pos != 0,
            [Atom::Star, tail @ (Atom::Text(_) | Atom::FoldedText(..))] => {
                if period && pos == 0 {
                    return false;
                }
                if let Ok(rest) = literal_edge(tail, &value[pos..], false) {
                    return rest.is_some_and(|s| !PATHNAME || !s.contains('/'));
                }
            }
            _ => {}
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
    insensitive_match_with(value, pattern, extended, locale, || {
        Pattern::insensitive(pattern, extended, locale).accepts(value, false)
    })
}

#[derive(Debug)]
pub(crate) enum Compiled {
    Basic(Matcher<String>),
    Extended(Pattern),
}

impl Compiled {
    pub(crate) fn new(source: &str, extended: bool) -> Self {
        if extended && has_group(source) { Self::Extended(Pattern::parse(source,true)) }
        else { Self::Basic(Matcher::new(source.to_owned())) }
    }

    pub(crate) fn accepts(&self, value: &str) -> bool {
        match self {
            Self::Basic(p) => p.matches(value),
            Self::Extended(p) => p.accepts(value,false),
        }
    }
}

pub(crate) fn session_match(value: &str, source: &str, extended: bool, session: &crate::Session) -> bool {
    let fold = session.shopt.nocasematch.then(|| session.character_locale());
    if let Some(fold) = fold {
        return insensitive_match_with(value, source, extended, fold, || {
            cached_match(value,source,extended,Some(fold),session)
        });
    }
    cached_match(value,source,extended,None,session)
}

pub(crate) fn insensitive_session(value: &str, source: &str, extended: bool, session: &crate::Session) -> bool {
    session_match(value,source,extended,session)
}

fn cached_match(value: &str, source: &str, extended: bool, fold: Option<super::CharacterLocale>, session: &crate::Session) -> bool {
    struct Entry {
        source: String,
        extended: bool,
        fold: Option<super::CharacterLocale>,
        locale: Option<std::rc::Rc<super::PatternLocale>>,
        ascii_ranges: bool,
        matcher: Compiled,
    }
    const LIMIT: usize = 12;
    thread_local! {static RECENT: std::cell::RefCell<Vec<Entry>> = const {std::cell::RefCell::new(Vec::new())};}
    let locale = (source.contains(['[','?']) || fold.is_some()).then(|| super::session_locale(session)).flatten();
    let _guard = locale.as_ref().and_then(|l| l.enter());
    let compile = || if let Some(fold) = fold { Compiled::Extended(Pattern::insensitive(source,extended,fold)) } else { Compiled::new(source,extended) };
    if source.len() > 4096 {
        return compile().accepts(value);
    }
    RECENT.with_borrow_mut(|recent| {
        if let Some(entry) = recent.iter().rev().find(|e| e.source == source && e.extended == extended && e.fold == fold && e.ascii_ranges == session.shopt.globasciiranges &&
            e.locale.as_ref().map(std::rc::Rc::as_ptr) == locale.as_ref().map(std::rc::Rc::as_ptr)) {
            return entry.matcher.accepts(value);
        }
        let matcher=compile();
        let yes=matcher.accepts(value);
        if recent.len() == LIMIT { recent.remove(0); }
        recent.push(Entry {source:source.into(),extended,fold,locale,ascii_ranges:session.shopt.globasciiranges,matcher});
        yes
    })
}

fn insensitive_match_with(
    value: &str,
    pattern: &str,
    extended: bool,
    locale: super::CharacterLocale,
    complex_match: impl FnOnce() -> bool,
) -> bool {
    let complex = pattern
        .bytes()
        .any(|b| matches!(b, b'?' | b'[' | b'\\' | 0))
        || extended && has_group(pattern);
    if !complex {
        match pattern.split_once('*') {
            None => return super::folded_prefix(value, pattern, locale) == Some(value.len()),
            Some((head, tail)) if !tail.contains('*') => {
                let Some(offset) = super::folded_prefix(value, head, locale) else {
                    return false;
                };
                return super::case::strip_folded_suffix(&value[offset..], tail, locale).is_some();
            }
            _ => {}
        }
    }
    complex_match()
}
