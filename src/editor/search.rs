use crate::{
    Session,
    history::{History, SearchQuery, View},
};

#[derive(Default)]
pub(super) struct ReverseSearch {
    pub(super) query: String,
    pub(super) index: usize,
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
