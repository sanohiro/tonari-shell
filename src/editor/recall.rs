use super::Action;

pub(super) struct Recall {
    index: usize,
    draft: String,
    cursor: usize,
    filter: Option<Matches>,
}

#[derive(Default)]
struct Matches {
    // Source indices, newest first. Inspect each skipped history row at most once.
    rows: Vec<usize>,
    position: usize,
    exhausted: bool,
}

impl Recall {
    pub(super) fn new(end: usize) -> Self {
        Self {
            index: end,
            draft: String::new(),
            cursor: 0,
            filter: None,
        }
    }

    pub(super) fn position(&self) -> usize {
        self.index
    }

    pub(super) fn on_match(&self, end: usize) -> bool {
        self.filter.is_some() && self.index < end
    }

    pub(super) fn edited(&mut self, end: usize) {
        if self.filter.take().is_some() {
            self.index = end;
        }
    }

    pub(super) fn navigate(
        &mut self,
        action: Action,
        lines: &[String],
        text: &mut String,
        cursor: &mut usize,
    ) {
        use Action::*;
        if !matches!(action, PrefixUp | PrefixDown) {
            self.filter = None;
        }
        let end = lines.len();
        if self.index == end && self.filter.is_none() {
            if matches!(action, Down | LastHistory | PrefixDown) {
                return;
            }
            self.draft.clone_from(text);
            self.cursor = *cursor;
            if action == PrefixUp {
                self.filter = Some(Matches::default());
            }
        }
        let older = matches!(action, Up | PrefixUp | FirstHistory);
        let next = match self.filter.as_mut() {
            Some(matches) if self.cursor != 0 => {
                if older {
                    if matches.position == matches.rows.len() && !matches.exhausted {
                        let prefix = &self.draft[..self.cursor];
                        match lines[..self.index]
                            .iter()
                            .rposition(|line| line.starts_with(prefix))
                        {
                            Some(i) => matches.rows.push(i),
                            None => matches.exhausted = true,
                        }
                    }
                    let Some(&i) = matches.rows.get(matches.position) else {
                        return;
                    };
                    matches.position += 1;
                    i
                } else {
                    matches.position = matches.position.saturating_sub(1);
                    matches
                        .position
                        .checked_sub(1)
                        .map_or(end, |pos| matches.rows[pos])
                }
            }
            _ if action == FirstHistory => 0,
            _ if action == LastHistory => end,
            _ if older => self.index.saturating_sub(1),
            _ => (self.index + 1).min(end),
        };
        if self.index != next {
            self.index = next;
            if let Some(line) = lines.get(next) {
                text.clone_from(line);
                *cursor = text.len();
            } else {
                text.clone_from(&self.draft);
                *cursor = self.cursor;
            }
        }
    }
}
