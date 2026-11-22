// Keep quote and expansion boundaries until brace alternatives have been chosen.
// Cloning a parameter or substitution here makes each chosen word evaluate it
// independently, in the same order as Bash.
use super::{Part, Word};
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

    fn values(&self) -> impl DoubleEndedIterator<Item = String> + '_ {
        let steps = self.first.abs_diff(self.last) / self.stride;
        (0..=steps).map(|index| {
            let delta = index * self.stride;
            let n = if self.first <= self.last {
                self.first.wrapping_add_unsigned(delta)
            } else {
                self.first.wrapping_sub_unsigned(delta)
            };
            if self.letters {
                if n == i64::from(b'\\') {
                    String::new()
                } else {
                    char::from(n as u8).to_string()
                }
            } else if self.width == 0 {
                n.to_string()
            } else {
                format!("{n:0width$}", width = self.width)
            }
        })
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
                let len = raw
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .count();
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

pub fn fields(parts: Vec<Part>, env: &mut Expansion<'_>, glob: bool) -> Result<Vec<String>, Error> {
    if let [Part::Range(range)] = &parts[..] {
        return Ok(range.values().collect());
    }
    let mut result = Vec::new();
    let mut pending = vec![parts];
    while let Some(mut parts) = pending.pop() {
        match choice(&parts) {
            Some(Choice::Range(i)) => {
                let Part::Range(range) = &parts[i] else {
                    unreachable!()
                };
                for value in range.values().rev() {
                    let mut word = parts[..i].to_vec();
                    word.push(if value.is_empty() {
                        Part::Text(value)
                    } else {
                        Part::Pattern(value)
                    });
                    word.extend_from_slice(&parts[i + 1..]);
                    pending.push(word);
                }
            }
            Some(Choice::Alternatives {
                open,
                close,
                commas,
            }) => {
                let mut end = close;
                for start in commas.into_iter().rev().chain([open]) {
                    let mut word = parts[..open].to_vec();
                    word.extend_from_slice(&parts[start + 1..end]);
                    word.extend_from_slice(&parts[close + 1..]);
                    if !word.is_empty() {
                        pending.push(word);
                    }
                    end = start;
                }
            }
            None => {
                if let Some(Part::BraceTilde(prefix)) = parts.first()
                    && (parts.len() == 1
                        || matches!(parts.get(1), Some(Part::Text(s) | Part::Pattern(s)) if s.starts_with('/')))
                {
                    parts[0] = Part::Tilde((*prefix).into());
                }
                result.extend(env.resolved_fields(Word::Expanded(parameter_names(parts)), glob)?);
            }
        }
    }
    Ok(result)
}
