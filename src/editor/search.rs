use crate::{
    Session,
    history::{History, SearchQuery, View},
};

#[derive(Default)]
pub(super) struct ReverseSearch {
    pub(super) query: String,
    index: usize,
    pub(super) original: String,
    pub(super) original_cursor: usize,
    pub(super) pending: Vec<SearchQuery>,
    matched: Option<String>,
}

impl ReverseSearch {
    pub(super) fn new(original: &str, original_cursor: usize) -> Self {
        Self {
            original: original.to_owned(),
            original_cursor,
            ..Self::default()
        }
    }

    pub(super) fn edit_query(
        &mut self,
        action: super::Action,
        first: u8,
        keys: &mut super::Keys,
        session: &Session,
        partial: &mut Vec<u8>,
        frame: impl FnMut(&[u8]) -> std::io::Result<()>,
    ) -> std::io::Result<Option<usize>> {
        use super::Action;
        let mut shortest = None;
        match action {
            Action::Search => self.index += 1,
            Action::Rubout => {
                self.query.pop();
                self.index = 0;
            }
            Action::Insert => {
                let start = self.query.len();
                keys.append_burst(first, session, &mut self.query)?;
                if let Some(c) = self.query[start..].chars().next() {
                    shortest = Some(start + c.len_utf8());
                    self.index = 0;
                }
            }
            Action::Paste => {
                self.query.push_str(&keys.resume_paste(partial, frame)?);
                self.index = 0;
            }
            _ => {}
        }
        Ok(shortest)
    }

    pub(super) fn wants_picker(&self, action: super::Action) -> bool {
        use super::Action;
        matches!(
            action,
            Action::Complete | Action::Up | Action::Down | Action::PrefixUp | Action::PrefixDown
        ) || (!self.query.chars().any(char::is_control)
            && crate::history::fragments(&self.query).nth(1).is_some())
    }

    pub(super) fn queue(&mut self, shortest: Option<usize>) {
        self.pending.push(SearchQuery {
            text: self.query.clone(),
            skip: self.index,
            shortest: shortest.unwrap_or(self.query.len()),
        });
    }

    pub(super) fn update(
        &mut self,
        text: &str,
        history: &mut History,
        session: &Session,
        view: &mut View,
        input_ready: &dyn Fn() -> bool,
    ) -> Option<String> {
        let last = self.pending.last()?;
        // If the newest hit still matches a longer query, it is still the newest.
        if last.skip == 0
            && self
                .matched
                .as_ref()
                .is_some_and(|prior| last.text.starts_with(prior) && text.contains(&last.text))
        {
            self.matched = self.pending.pop().map(|q| q.text);
            self.pending.clear();
            return None;
        }

        // A failed or interrupted lookup leaves its requests available to retry.
        let result = history
            .find_command(session, view, &self.pending, input_ready)
            .ok()?;
        let last = self.pending.pop()?;
        self.matched = result
            .as_ref()
            .filter(|hit| last.skip == 0 && hit.contains(&last.text))
            .map(|_| last.text);
        self.pending.clear();
        result.filter(|found| found != text)
    }

    pub(super) fn prompt(&self, width: usize) -> String {
        let query = &self.query;
        let mut display = String::from("(reverse-i-search) `");
        let visible = if query.len() > super::VIEWPORT_THRESHOLD {
            display.push('…');
            let mut start = query.len().saturating_sub(width * 2);
            while !query.is_char_boundary(start) {
                start += 1;
            }
            &query[start..]
        } else {
            query
        };
        super::push_visible(&mut display, visible);
        display.push_str("': ");
        display
    }
}
