use super::{Atom, Pattern, one_character};
use crate::pattern::{CharacterLocale, ParameterPattern};

pub(crate) const MISS: usize = usize::MAX;

// Each row gives the best end at every starting offset of a suffix pattern.
// Compute rows backwards to share work between successive search positions.
pub(crate) fn ascii_ends(
    pattern: &ParameterPattern<'_>,
    text: &str,
    anchored: bool,
    longest: bool,
) -> Option<Vec<usize>> {
    if text.len() < 256
        || !pattern.text.contains(['*', '+'])
        || !pattern.text.is_ascii()
        || !text.is_ascii()
        || text.as_bytes().contains(&0)
        || pattern.fold == Some(CharacterLocale::Turkic)
        || pattern.text.contains("[.")
        || pattern.text.contains("[=")
    {
        return None;
    }
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
    let mut ends: Vec<_> = (0..=text.len())
        .map(|i| if anchored && i != text.len() { MISS } else { i })
        .collect();
    let bytes = text.as_bytes();
    for atom in atoms.iter().rev() {
        let (mask, repeat, optional) = match atom {
            Atom::Text(s) | Atom::FoldedText(s, _) => {
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
            Atom::Star => (u128::MAX, true, true),
            Atom::Any | Atom::Class(..) | Atom::CharacterClass(_) => {
                (unit_mask(atom)?, false, false)
            }
            Atom::Group(op @ (b'+' | b'*'), options) => {
                let mut letters = 0;
                for option in options {
                    let [unit] = option.as_slice() else {
                        return None;
                    };
                    letters |= unit_mask(unit)?;
                }
                (letters, true, *op == b'*')
            }
            _ => return None,
        };
        let mut stop = ends[text.len()];
        if !optional {
            ends[text.len()] = MISS;
        }
        for (i, &byte) in bytes.iter().enumerate().rev() {
            let tail = ends[i];
            let take = if mask & (1 << byte) == 0 { MISS } else { stop };
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
