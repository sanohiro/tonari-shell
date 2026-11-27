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

    fn value(&self, index: u64) -> String {
        let delta = index * self.stride;
        let n = if self.first <= self.last {
            self.first.wrapping_add_unsigned(delta)
        } else {
            self.first.wrapping_sub_unsigned(delta)
        };
        if self.letters {
            char::from(n as u8).to_string()
        } else if self.width == 0 {
            n.to_string()
        } else {
            format!("{n:0width$}", width = self.width)
        }
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
    for (open, part) in parts.iter().enumerate() {
        match part {
            Part::Range(_) => return Some(Choice::Range(open)),
            Part::Brace("{") => {
                let mut depth = 0;
                let mut commas = Vec::new();
                for (close, part) in parts.iter().enumerate().skip(open + 1) {
                    match part {
                        Part::Brace("{") => depth += 1,
                        Part::Brace("}") if depth > 0 => depth -= 1,
                        Part::Brace("}") => {
                            if !commas.is_empty() {
                                return Some(Choice::Alternatives {
                                    open,
                                    close,
                                    commas,
                                });
                            }
                            break;
                        }
                        Part::Brace(",") if depth == 0 => commas.push(close),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    None
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
                name.push_str(&raw[..len]);
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
    if word_count(&parts) > MAX_WORDS {
        return Err("brace expansion exceeds word limit".into());
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
