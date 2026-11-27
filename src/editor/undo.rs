use std::{collections::VecDeque, ops::Range};

const MAX_UNDO_ENTRIES: usize = 128;
const MAX_UNDO_BYTES: usize = 4 * 1024 * 1024;

struct Change {
    inserted: Range<usize>,
    removed: String,
    cursor: usize,
}

// Only this readline invocation owns its edits. Keep one oversized removal undoable.
#[derive(Default)]
pub(super) struct Undo {
    entries: VecDeque<Change>,
    bytes: usize,
    typing: bool,
}

impl Undo {
    pub(super) fn boundary(&mut self) {
        self.typing = false;
    }

    pub(super) fn record(
        &mut self,
        line: &str,
        range: Range<usize>,
        added: usize,
        cursor: usize,
        typing: bool,
    ) {
        if range.is_empty() && added == 0 {
            return;
        }
        let append = typing && range.is_empty();
        if append
            && self.typing
            && let Some(last) = self.entries.back_mut()
            && last.inserted.end == range.start
        {
            last.inserted.end += added;
            return;
        }
        self.typing = append;
        let removed = &line[range.clone()];
        while !self.entries.is_empty()
            && (self.entries.len() >= MAX_UNDO_ENTRIES
                || self.bytes.saturating_add(removed.len()) > MAX_UNDO_BYTES)
        {
            self.bytes -= self.entries.pop_front().unwrap().removed.len();
        }
        self.bytes += removed.len();
        self.entries.push_back(Change {
            inserted: range.start..range.start + added,
            removed: removed.to_owned(),
            cursor,
        });
    }

    pub(super) fn insert(&mut self, line: &mut String, cursor: &mut usize, value: &str) {
        self.record(line, *cursor..*cursor, value.len(), *cursor, false);
        line.insert_str(*cursor, value);
        *cursor += value.len();
    }

    /// Completion and bind -x can return an entire word or line for a small edit.
    pub(super) fn replace(
        &mut self,
        line: &mut String,
        range: Range<usize>,
        new: &str,
        cursor: usize,
    ) {
        let old = &line[range.clone()];
        let mut prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let mut suffix = old[prefix..]
            .bytes()
            .rev()
            .zip(new[prefix..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(old.len() - suffix) {
            suffix -= 1;
        }
        let changed = range.start + prefix..range.end - suffix;
        let replacement = &new[prefix..new.len() - suffix];
        self.record(line, changed.clone(), replacement.len(), cursor, false);
        line.replace_range(changed, replacement);
    }

    pub(super) fn apply(&mut self, line: &mut String, cursor: &mut usize) {
        self.boundary();
        if let Some(change) = self.entries.pop_back() {
            self.bytes -= change.removed.len();
            line.replace_range(change.inserted, &change.removed);
            *cursor = change.cursor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appending_keeps_only_offsets_and_coalesces_adjacent_typing() {
        let mut line = "a".repeat(1024 * 1024);
        let mut undo = Undo::default();
        let start = line.len();
        for c in ['猫', '🌱'] {
            let at = line.len();
            undo.record(&line, at..at, c.len_utf8(), at, true);
            line.push(c);
        }
        assert_eq!((undo.entries.len(), undo.bytes), (1, 0));
        let mut cursor = line.len();
        undo.apply(&mut line, &mut cursor);
        assert_eq!((cursor, line.len()), (start, start));
    }

    #[test]
    fn diff_preserves_utf8_boundaries_and_both_unchanged_ends() {
        let mut line = "path/早🌱-end".to_owned();
        let len = line.len();
        let mut undo = Undo::default();
        undo.replace(&mut line, 0..len, "path/日🌱-end", 5);
        assert_eq!(undo.bytes, "早".len());
        let mut cursor = line.len();
        undo.apply(&mut line, &mut cursor);
        assert_eq!((line.as_str(), cursor), ("path/早🌱-end", 5));
    }
}
