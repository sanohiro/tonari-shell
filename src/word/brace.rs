// Keep quote and expansion boundaries until brace alternatives have been chosen.
// Cloning a parameter or substitution here makes each chosen word evaluate it
// independently, in the same order as Bash.
use super::{Part, Word, name_chars_len};
use crate::expansion::{Error, Expansion};

#[derive(Clone, Debug)]
pub struct Range {
    pub spelling: String,
    first: i64,
    last: i64,
    stride: u64,
    width: usize,
    letters: bool,
}

impl Range {
    pub fn parse(inner: &str) -> Option<Self> {
        let mut items = inner.split("..");
        let first = items.next()?;
        let last = items.next()?;
        let stride = match items.next() {
            Some(text) => text.parse::<i64>().ok()?.unsigned_abs().max(1),
            None => 1,
        };
        if items.next().is_some() {
            return None;
        }
        let letter = |s: &str| s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic();
        let padded = |s: &str| {
            let digits = s.strip_prefix('-').unwrap_or(s);
            digits.len() > 1 && digits.starts_with('0')
        };
        let (first, last, width, letters): (i64, i64, usize, bool) =
            if letter(first) && letter(last) {
                (
                    first.as_bytes()[0] as i64,
                    last.as_bytes()[0] as i64,
                    0,
                    true,
                )
            } else {
                let width = if padded(first) || padded(last) {
                    first.len().max(last.len())
                } else {
                    0
                };
                (first.parse().ok()?, last.parse().ok()?, width, false)
            };
        let distance = i64::try_from(first.abs_diff(last)).ok()?;
        let stride_signed = i64::try_from(stride).ok()?;
        (distance / stride_signed).checked_add(2)?;
        Some(Self {
            spelling: format!("{{{inner}}}"),
            first,
            last,
            stride,
            width,
            letters,
        })
    }

    fn count(&self) -> u64 {
        1 + self.first.abs_diff(self.last) / self.stride
    }

    fn number(&self, index: u64) -> i64 {
        let delta = index * self.stride;
        if self.first <= self.last {
            self.first.wrapping_add_unsigned(delta)
        } else {
            self.first.wrapping_sub_unsigned(delta)
        }
    }

    fn append_value(&self, index: u64, word: &mut String) {
        use std::fmt::Write;
        let n = self.number(index);
        if self.letters {
            word.push(char::from(n as u8));
        } else if self.width == 0 {
            word.push_str(itoa::Buffer::new().format(n));
        } else {
            write!(word, "{n:0width$}", width = self.width).expect("in-memory write");
        }
    }

    fn value(&self, index: u64) -> String {
        let mut text = String::new();
        self.append_value(index, &mut text);
        text
    }
}

enum Choice {
    Alternatives {
        open: usize,
        close: usize,
        commas: Vec<usize>,
    },
    Range(usize),
}

fn choice(parts: &[Part]) -> Option<Choice> {
    if !parts.iter().any(|p| matches!(p, Part::Brace("}"))) {
        return parts
            .iter()
            .position(|p| matches!(p, Part::Range(_)))
            .map(Choice::Range);
    }
    let mut stack = Vec::<(usize, Vec<usize>)>::new();
    let mut first = None;
    let mut best = usize::MAX;
    for (index, part) in parts.iter().enumerate() {
        match part {
            Part::Brace("{") => stack.push((index, Vec::new())),
            Part::Brace(",") => {
                if let Some((_, commas)) = stack.last_mut() {
                    commas.push(index);
                }
            }
            Part::Brace("}") => {
                if let Some((open, commas)) = stack.pop()
                    && open < best
                    && !commas.is_empty()
                {
                    best = open;
                    first = Some(Choice::Alternatives {
                        open,
                        close: index,
                        commas,
                    });
                }
            }
            Part::Range(_) if index < best => {
                best = index;
                first = Some(Choice::Range(index));
            }
            _ => {}
        }
        if first.is_some() && stack.is_empty() {
            break;
        }
    }
    first
}

// Unquoted $name{x,y} spells $namex and $namey after brace expansion.
// Explicit ${name}, quoted letters and escaped characters keep their boundaries.
fn parameter_names(parts: Vec<Part>) -> Vec<Part> {
    if !parts.iter().any(|p| matches!(p, Part::BraceParameter(_))) {
        return parts;
    }
    let mut result = Vec::with_capacity(parts.len());
    let mut parts = parts.into_iter().peekable();
    while let Some(part) = parts.next() {
        if let Part::BraceParameter(mut name) = part {
            while let Some(Part::Pattern(raw)) = parts.peek_mut() {
                let len = name_chars_len(raw);
                if len == 0 {
                    break;
                }
                name.to_mut().push_str(&raw[..len]);
                if len != raw.len() {
                    raw.drain(..len);
                    break;
                }
                parts.next();
            }
            result.push(Part::Parameter {
                name,
                quoted: false,
            });
        } else {
            result.push(part);
        }
    }
    result
}

const MAX_WORDS: u64 = 1 << 20;
const MAX_BYTES: usize = 64 << 20;

// Count alternatives with sums and adjacent expansions with products. An
// unmatched opening brace leaves its commas literal, but ranges still expand.
fn word_count(parts: &[Part]) -> u64 {
    #[derive(Default)]
    struct Group {
        sum: u64,
        product: u64,
    }
    let mut parents = Vec::new();
    let mut group = Group::default();
    let mut term = 1u64;
    for part in parts {
        match part {
            Part::Range(range) => term = term.saturating_mul(range.count()),
            Part::Brace("{") => {
                parents.push((group, term));
                group = Group { sum: 0, product: 1 };
                term = 1;
            }
            Part::Brace(",") if !parents.is_empty() => {
                group.sum = group.sum.saturating_add(term);
                group.product = group.product.saturating_mul(term);
                term = 1;
            }
            Part::Brace("}") if !parents.is_empty() => {
                let alternatives = group.sum.saturating_add(term);
                let (outer, prefix) = parents.pop().unwrap();
                group = outer;
                term = prefix.saturating_mul(alternatives);
            }
            _ => {}
        }
    }
    while let Some((outer, prefix)) = parents.pop() {
        term = term.saturating_mul(group.product).saturating_mul(prefix);
        group = outer;
    }
    term
}

enum Siblings {
    Range {
        index: usize,
        next: u64,
    },
    Alternatives {
        open: usize,
        start: usize,
        ends: std::vec::IntoIter<usize>,
        close: usize,
    },
}

impl Siblings {
    // Materialize only the next sibling. Large ranges do not retain a copy
    // of their prefix, suffix and nested substitutions for every pending word.
    fn next(&mut self, parts: &[Part]) -> Option<Vec<Part>> {
        match self {
            Self::Range { index, next } => {
                let i = *index;
                let Part::Range(range) = &parts[i] else {
                    unreachable!()
                };
                if *next >= range.count() {
                    return None;
                }
                let value = range.value(*next);
                *next += 1;
                let mut word = Vec::with_capacity(parts.len());
                word.extend_from_slice(&parts[..i]);
                word.push(if value.as_bytes()[0].is_ascii_alphanumeric() {
                    Part::Pattern(value)
                } else {
                    Part::Text(value)
                });
                word.extend_from_slice(&parts[i + 1..]);
                Some(word)
            }
            Self::Alternatives {
                open,
                start,
                ends,
                close,
            } => {
                let end = ends.next()?;
                let begin = *start;
                *start = end + 1;
                let mut word = Vec::with_capacity(*open + end - begin + parts.len() - *close - 1);
                word.extend_from_slice(&parts[..*open]);
                word.extend_from_slice(&parts[begin..end]);
                word.extend_from_slice(&parts[*close + 1..]);
                Some(word)
            }
        }
    }
}

// Restrict the product to inert fragments. Quoting, splitting, pathname
// patterns and mixed-case letter ranges keep their existing expansion path.
fn literal_fragment(parts: &[Part]) -> Option<&str> {
    match parts {
        [] => Some(""),
        [Part::Text(s) | Part::Pattern(s)]
            if s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')) =>
        {
            Some(s)
        }
        _ => None,
    }
}

fn append_range_product(parts: &[Part], count: usize, result: &mut Vec<String>) -> Option<()> {
    let mut ranges = parts.iter().enumerate().filter_map(|(index, part)| {
        if let Part::Range(range) = part {
            Some((index, range))
        } else {
            None
        }
    });
    let (i, left) = ranges.next()?;
    let (j, right) = ranges.next()?;
    let prefix = literal_fragment(&parts[..i])?;
    let middle = literal_fragment(&parts[i + 1..j])?;
    let suffix = literal_fragment(&parts[j + 1..])?;
    if [left, right].iter().any(|r| {
        r.letters && (r.first as u8).is_ascii_uppercase() != (r.last as u8).is_ascii_uppercase()
    }) {
        return None;
    }
    // A signed i64 needs at most 20 decimal bytes before zero padding.
    // Near the byte limit, fall back to accounting for actual expanded fields.
    let max_word = (prefix.len() + middle.len() + suffix.len())
        .saturating_add(left.width.max(20))
        .saturating_add(right.width.max(20));
    if max_word.saturating_mul(count) > MAX_BYTES {
        return None;
    }
    result.reserve(count);
    let start = result.len();
    let mut head = [prefix, &left.value(0), middle].concat();
    let head_len = head.len();
    for n in 0..right.count() {
        result.push([&head, right.value(n).as_str(), suffix].concat());
    }
    // Reuse the tails in the first row, including when this word was appended
    // after earlier arguments. No auxiliary copy of the inner range is needed.
    let first_row = start..result.len();
    for n in 1..left.count() {
        head.truncate(prefix.len());
        left.append_value(n, &mut head);
        head.push_str(middle);
        for index in first_row.clone() {
            let word = [head.as_str(), &result[index][head_len..]].concat();
            result.push(word);
        }
    }
    Some(())
}

// Link the alternatives before emitting any fields. An unquoted empty branch,
// a mixed-case letter range or a fragment needing expansion takes the general path.
fn literal_product(parts: &[Part], count: usize, result: &mut Vec<String>) -> Option<()> {
    if parts.len() > 128 || count < 2 {
        return None;
    }
    let mut next = vec![0; parts.len()];
    let mut groups = Vec::new();
    let mut limit = 0usize;
    for (i, part) in parts.iter().enumerate() {
        limit = limit.saturating_add(match part {
            Part::Text(s) | Part::Pattern(s)
                if !s.is_empty() && literal_fragment(std::slice::from_ref(part)).is_some() =>
            {
                s.len()
            }
            Part::Range(r)
                if !r.letters
                    || (r.first as u8).is_ascii_uppercase()
                        == (r.last as u8).is_ascii_uppercase() =>
            {
                r.width.max(20)
            }
            Part::Brace("{") => {
                groups.push((i, i));
                0
            }
            Part::Brace(",") => {
                let (_, prev) = groups.last_mut()?;
                if *prev + 1 == i {
                    return None;
                }
                next[*prev] = i;
                *prev = i;
                0
            }
            Part::Brace("}") => {
                let (open, prev) = groups.pop()?;
                if open == prev || prev + 1 == i {
                    return None;
                }
                next[prev] = i;
                0
            }
            _ => return None,
        });
    }
    if !groups.is_empty() || limit.saturating_mul(count) > MAX_BYTES {
        return None;
    }
    fn emit(
        parts: &[Part],
        next: &[usize],
        mut i: usize,
        word: &mut String,
        out: &mut Vec<String>,
    ) {
        let start = word.len();
        while let Some(part) = parts.get(i) {
            match part {
                Part::Pattern(s) | Part::Text(s) => word.push_str(s),
                Part::Range(r) => {
                    let end = word.len();
                    for j in 0..r.count() {
                        r.append_value(j, word);
                        emit(parts, next, i + 1, word, out);
                        word.truncate(end);
                    }
                    break;
                }
                Part::Brace("{") => {
                    loop {
                        emit(parts, next, i + 1, word, out);
                        i = next[i];
                        if matches!(parts[i], Part::Brace("}")) {
                            break;
                        }
                    }
                    break;
                }
                Part::Brace(",") => {
                    while matches!(parts[i], Part::Brace(",")) {
                        i = next[i];
                    }
                }
                Part::Brace("}") => {}
                _ => unreachable!(),
            }
            i += 1;
        }
        if i == parts.len() {
            out.push(word.clone());
        }
        word.truncate(start);
    }
    result.reserve(count);
    emit(parts, &next, 0, &mut String::with_capacity(limit), result);
    Some(())
}

// Repeated commands own the resulting argument strings. An inert brace tree can
// emit them directly from the borrowed syntax while the session has braceexpand on.
pub(crate) fn append_borrowed_product(parts: &[Part], out: &mut Vec<String>, limit: usize) -> bool {
    if parts.len() < 3
        || !parts
            .iter()
            .any(|p| matches!(p, Part::Brace("{") | Part::Range(_)))
    {
        return false;
    }
    let count = word_count(parts);
    count <= MAX_WORDS && count <= limit as u64 && append_product(parts, count as usize, out)
}

fn append_product(parts: &[Part], count: usize, out: &mut Vec<String>) -> bool {
    append_range_product(parts, count, out).is_some()
        || literal_product(parts, count, out).is_some()
}

pub fn append_fields(
    parts: Vec<Part>,
    env: &mut Expansion<'_>,
    glob: bool,
    result: &mut Vec<String>,
) -> Result<(), Error> {
    if let [Part::Range(range)] = &parts[..] {
        let count = range.count();
        if count > MAX_WORDS {
            return Err("brace expansion exceeds word limit".into());
        }
        if range.width.saturating_mul(count as usize) > MAX_BYTES {
            return Err("brace expansion exceeds byte limit".into());
        }
        result.reserve(count as usize);
        result.extend((0..count).map(|i| range.value(i)));
        return Ok(());
    }
    let count = word_count(&parts);
    if count > MAX_WORDS {
        return Err("brace expansion exceeds word limit".into());
    }
    if append_product(&parts, count as usize, result) {
        return Ok(());
    }
    let mut pending = Vec::new();
    let mut word = Some(parts);
    let mut bytes = 0usize;
    loop {
        if let Some(mut parts) = word.take() {
            match choice(&parts) {
                Some(Choice::Range(index)) => {
                    pending.push((parts, Siblings::Range { index, next: 0 }))
                }
                Some(Choice::Alternatives {
                    open,
                    close,
                    mut commas,
                }) => {
                    commas.push(close);
                    pending.push((
                        parts,
                        Siblings::Alternatives {
                            open,
                            start: open + 1,
                            ends: commas.into_iter(),
                            close,
                        },
                    ));
                }
                None => {
                    resolve_tilde(&mut parts);
                    let before = result.len();
                    env.append_resolved_fields(
                        Word::Expanded(parameter_names(parts)),
                        glob,
                        result,
                    )?;
                    for field in &result[before..] {
                        bytes = bytes.saturating_add(field.len());
                    }
                    if bytes > MAX_BYTES {
                        return Err("brace expansion exceeds byte limit".into());
                    }
                }
            }
        }
        let Some((parts, siblings)) = pending.last_mut() else {
            return Ok(());
        };
        match siblings.next(parts) {
            Some(next) if !next.is_empty() => word = Some(next),
            Some(_) => {}
            None => {
                pending.pop();
            }
        }
    }
}

fn resolve_tilde(parts: &mut Vec<Part>) {
    let Some(Part::BraceTilde(prefix)) = parts.first() else {
        return;
    };
    let mut name = prefix.to_string();
    let mut end = 1;
    while let Some(part) = parts.get(end) {
        let Part::Pattern(raw) = part else { return };
        let login = raw.split('/').next().unwrap();
        if !login
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '+' | '.' | '@'))
        {
            return;
        }
        name.push_str(login);
        if login.len() < raw.len() {
            let n = login.len();
            if let Part::Pattern(raw) = &mut parts[end] {
                raw.drain(..n);
            }
            break;
        }
        end += 1;
    }
    parts[0] = Part::Tilde(name);
    parts.drain(1..end);
}
