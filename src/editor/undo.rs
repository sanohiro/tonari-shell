use std::{collections::VecDeque, ops::Range};

// Only this readline invocation owns its edits. Keep one oversized edit undoable.
#[derive(Default)]
pub(super) struct Undo {
    entries: VecDeque<(String, usize)>,
    bytes: usize,
    typing_end: Option<usize>,
}

impl Undo {
    pub(super) fn boundary(&mut self) {
        self.typing_end = None;
    }

    pub(super) fn record(
        &mut self,
        line: &str,
        range: Range<usize>,
        added: usize,
        point: usize,
        typing: bool,
    ) {
        if range.is_empty() && added == 0 {
            return;
        }
        let joined = typing && range.is_empty() && self.typing_end == Some(range.start);
        self.typing_end = (typing && range.is_empty()).then_some(range.start + added);
        if joined {
            return;
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= 128
                || self.bytes.saturating_add(line.len()) > 4 * 1024 * 1024)
        {
            self.bytes -= self.entries.pop_front().unwrap().0.len();
        }
        self.bytes += line.len();
        self.entries.push_back((line.to_owned(), point));
    }

    pub(super) fn apply(&mut self, line: &mut String, cursor: &mut usize) {
        self.boundary();
        if let Some((old, point)) = self.entries.pop_back() {
            self.bytes -= old.len();
            *line = old;
            *cursor = point;
        }
    }
}
