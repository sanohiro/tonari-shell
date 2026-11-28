use super::{Action, Keymap, Session, Undo, next, previous};

#[derive(Default)]
pub(super) struct State {
    count: usize,
    operator: Option<(Action, usize)>,
}

fn class(cluster: &str, big: bool) -> u8 {
    let c = cluster.chars().next().unwrap();
    if c.is_whitespace() {
        0
    } else if big || c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

use unicode_segmentation::UnicodeSegmentation;

fn word(text: &str, at: usize, action: Action, big: bool) -> usize {
    if action == Action::ViPrevWord {
        let mut iter = text[..at]
            .grapheme_indices(true)
            .rev()
            .skip_while(|(_, c)| c.starts_with(char::is_whitespace));
        let Some((mut start, c)) = iter.next() else {
            return 0;
        };
        let kind = class(c, big);
        for (i, _) in iter.take_while(|(_, c)| class(c, big) == kind) {
            start = i;
        }
        return start;
    }
    if action == Action::ViEndWord {
        let after = next(text, at);
        let mut iter = text[after..]
            .grapheme_indices(true)
            .skip_while(|(_, c)| c.starts_with(char::is_whitespace));
        let Some((mut last, c)) = iter.next() else {
            return previous(text, text.len());
        };
        let kind = class(c, big);
        for (i, _) in iter.take_while(|(_, c)| class(c, big) == kind) {
            last = i;
        }
        return after + last;
    }
    let mut iter = text[at..].graphemes(true).peekable();
    let Some(&first) = iter.peek() else { return at };
    let kind = class(first, big);
    let mut end = at;
    while iter.peek().is_some_and(|&c| class(c, big) == kind) {
        end += iter.next().unwrap().len();
    }
    while iter
        .peek()
        .is_some_and(|c| c.starts_with(char::is_whitespace))
    {
        end += iter.next().unwrap().len();
    }
    end
}

impl State {
    pub(super) fn apply(
        &mut self,
        action: Action,
        first: u8,
        text: &mut String,
        cursor: &mut usize,
        session: &mut Session,
        undo: &mut Undo,
    ) -> Option<bool> {
        use Action::*;
        if action == ViDigit || (action == Home && first == b'0' && self.count != 0) {
            if first.is_ascii_digit() {
                self.count = self
                    .count
                    .saturating_mul(10)
                    .saturating_add((first - b'0') as usize);
            }
            return Some(false);
        }
        let count = std::mem::take(&mut self.count).max(1);
        let pending = self.operator.take();
        let operator_key = matches!(action, ViChange | ViDelete | ViYank);
        if operator_key && pending.is_none() && !first.is_ascii_uppercase() {
            self.operator = Some((action, count));
            return Some(false);
        }
        let mut operator = pending
            .map(|p| p.0)
            .or_else(|| operator_key.then_some(action));
        let repeat = pending.map_or(count, |p| count.saturating_mul(p.1));
        let mut start = *cursor;
        let mut end = start;
        if action == ViSubstitute {
            operator = Some(ViChange);
            if first == b'S' {
                start = 0;
                end = text.len();
            } else {
                for _ in 0..repeat {
                    if end == text.len() {
                        break;
                    }
                    end = next(text, end);
                }
            }
        } else if operator_key {
            if let Some((op, _)) = pending {
                if op != action {
                    return Some(false);
                }
                start = 0;
            }
            end = text.len();
        } else {
            for _ in 0..repeat {
                let pos = match action {
                    Back | Rubout => previous(text, end),
                    Forward | Delete => next(text, end),
                    Home => 0,
                    ViFirstPrint => text.len() - text.trim_start().len(),
                    End => text.len(),
                    ViNextWord | ViPrevWord | ViEndWord => {
                        word(text, end, action, first.is_ascii_uppercase())
                    }
                    _ => return pending.map(|_| false),
                };
                if pos == end {
                    break;
                }
                end = pos;
            }
            if operator.is_none() && !matches!(action, Delete | Rubout) {
                *cursor = end;
                return Some(true);
            }
            if action == ViEndWord {
                end = next(text, end);
            }
            if operator == Some(ViChange)
                && action == ViNextWord
                && start < end
                && !text[start..].starts_with(char::is_whitespace)
            {
                end = start + text[start..end].trim_end().len();
            }
        }
        let range = start.min(end)..end.max(start);
        if !range.is_empty() {
            session
                .editor_kill
                .entry(false)
                .push_str(&text[range.clone()]);
            if operator != Some(ViYank) {
                undo.record(text, range.clone(), 0, *cursor, false);
                *cursor = range.start;
                text.replace_range(range, "");
            }
        }
        if operator == Some(ViChange) {
            session.bindings.map = Keymap::Insert;
        }
        Some(false)
    }
}
