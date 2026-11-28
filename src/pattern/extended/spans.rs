use super::{Atom, Pattern, one_character};
use crate::pattern::{CharacterLocale, ParameterPattern};

pub(crate) const MISS: usize = usize::MAX;

// Each row gives the best end at every starting offset of a suffix pattern.
// Compute rows backwards to share work between successive search positions.
pub(crate) fn match_ends(
    pattern: &ParameterPattern<'_>,
    text: &str,
    anchored: bool,
    longest: bool,
) -> Option<Vec<usize>> {
    if text.len() < 256
        || !pattern.text.contains(['*', '+'])
        || text.as_bytes().contains(&0)
        || pattern.text.contains("[.")
        || pattern.text.contains("[=")
    {
        return None;
    }
    let ascii =
        text.is_ascii() && pattern.text.is_ascii() && pattern.fold != Some(CharacterLocale::Turkic);
    let parsed;
    let group = if let Some(group) = &pattern.group {
        group
    } else {
        parsed = Pattern::parse(pattern.text, false);
        &parsed
    };
    let atoms = &group.atoms;
    // A star adjacent to an extended group has special empty-match rules.
    if atoms.iter().any(|a| matches!(a, Atom::Star))
        && atoms
            .iter()
            .any(|a| matches!(a, Atom::Group(..) | Atom::Choice(_)))
    {
        return None;
    }
    // A mandatory literal outside any group must occur in every match.
    // An empty table records a proven miss without allocating a full row.
    if let Some(lit @ (Atom::Text(s) | Atom::FoldedText(s, _))) = atoms.last()
        && !s.is_empty()
    {
        let hay = text.as_bytes();
        let needle = s.as_bytes();
        let folded = matches!(lit, Atom::FoldedText(..));
        let possible = if folded && !ascii {
            let Atom::FoldedText(_, locale) = lit else {
                unreachable!()
            };
            if anchored {
                super::literal_edge(lit, text, false).ok()?.is_some()
            } else {
                text.char_indices()
                    .any(|(i, _)| crate::pattern::folded_prefix(&text[i..], s, *locale).is_some())
            }
        } else if anchored {
            hay.len() >= needle.len()
                && if folded {
                    hay[hay.len() - needle.len()..].eq_ignore_ascii_case(needle)
                } else {
                    hay.ends_with(needle)
                }
        } else if !folded {
            memchr::memmem::find(hay, needle).is_some()
        } else {
            let first = needle[0];
            memchr::memchr2(first.to_ascii_uppercase(), first.to_ascii_lowercase(), hay).is_some()
        };
        if !possible {
            return Some(Vec::new());
        }
    }
    let mut ends: Vec<_> = (0..=text.len())
        .map(|i| {
            if !text.is_char_boundary(i) || anchored && i != text.len() {
                MISS
            } else {
                i
            }
        })
        .collect();
    let bytes = text.as_bytes();
    for atom in atoms.iter().rev() {
        let (repeat, optional) = match atom {
            Atom::Text(s) | Atom::FoldedText(s, _) => {
                if !ascii && let Atom::FoldedText(needle, locale) = atom {
                    for (i, _) in text.char_indices() {
                        ends[i] = crate::pattern::folded_prefix(&text[i..], needle, *locale).map_or(MISS, |n| ends[i+n]);
                    }
                    ends[text.len()] = MISS;
                    continue;
                }
                for i in 0..ends.len() {
                    ends[i] = bytes
                        .get(i..i + s.len())
                        .filter(|candidate| {
                            if matches!(atom, Atom::FoldedText(..)) {
                                candidate.eq_ignore_ascii_case(s.as_bytes())
                            } else {
                                *candidate == s.as_bytes()
                            }
                        })
                        .map_or(MISS, |_| ends[i + s.len()]);
                }
                continue;
            }
            Atom::Star => (true, true),
            Atom::Any | Atom::Class(..) | Atom::CharacterClass(_) => (false, false),
            Atom::Group(op @ (b'+' | b'*'), options) if options.iter().all(|seq| {
                matches!(seq.as_slice(), [Atom::Any | Atom::Class(..) | Atom::CharacterClass(_)]) ||
                matches!(seq.as_slice(), [Atom::Text(s) | Atom::FoldedText(s, _)] if s.chars().count()==1)
            }) => (true, *op == b'*'),
            _ => return None,
        };
        let mut stop = ends[text.len()];
        if !optional {
            ends[text.len()] = MISS;
        }
        let mut step = |i: usize, accepts: bool| {
            let tail = ends[i];
            let take = if accepts { stop } else { MISS };
            stop = if !repeat || take == MISS {
                tail
            } else if tail == MISS {
                take
            } else if longest {
                take.max(tail)
            } else {
                take.min(tail)
            };
            ends[i] = if optional { stop } else { take };
        };
        if ascii {
            let mask = match atom {
                Atom::Star => u128::MAX,
                Atom::Group(_, options) => options
                    .iter()
                    .try_fold(0, |m, seq| unit_mask(&seq[0]).map(|u| m | u))?,
                unit => unit_mask(unit)?,
            };
            for (i, b) in bytes.iter().enumerate().rev() {
                step(i, mask & (1 << b) != 0);
            }
        } else {
            // A native basic '?' can have byte-oriented semantics in the C locale.
            if pattern.group.is_none() && matches!(atom, Atom::Any) {
                return None;
            }
            crate::pattern::localized(|| {
                for (i, c) in text.char_indices().rev() {
                    let unit = |a: &Atom| match a {
                        Atom::Text(s) => s.starts_with(c),
                        Atom::FoldedText(s, locale) => s
                            .chars()
                            .next()
                            .is_some_and(|ch| super::super::same_letter(c, ch, *locale)),
                        _ => one_character::<false>(a, text, i).is_some(),
                    };
                    let ok = match atom {
                        Atom::Star => true,
                        Atom::Group(_, alternatives) => alternatives.iter().any(|s| unit(&s[0])),
                        a => unit(a),
                    };
                    step(i, ok);
                }
            });
        }
    }
    Some(ends)
}

fn unit_mask(atom: &Atom) -> Option<u128> {
    match atom {
        Atom::Any => Some(u128::MAX),
        Atom::Text(s) | Atom::FoldedText(s, _) if s.len() == 1 => {
            let b = s.as_bytes()[0];
            let mut bits = 1 << b;
            if matches!(atom, Atom::FoldedText(..)) && b.is_ascii_alphabetic() {
                bits |= 1 << (b ^ 0x20);
            }
            Some(bits)
        }
        Atom::Class(..) | Atom::CharacterClass(_) => Some(crate::pattern::localized(|| {
            let mut bits = 0;
            for b in 1..=127u8 {
                let buf = [b];
                if one_character::<false>(atom, std::str::from_utf8(&buf).unwrap(), 0).is_some() {
                    bits |= 1 << b;
                }
            }
            bits
        })),
        _ => None,
    }
}
