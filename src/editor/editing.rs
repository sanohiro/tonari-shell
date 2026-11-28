use super::{
    Action, EditorState, Input, Session, char_offset, next, previous, word_end, word_start,
};

pub(super) fn edit_and_execute(
    session: &mut Session,
    mut text: String,
    vi: bool,
    status: u8,
    queued: std::collections::VecDeque<u8>,
) -> Input {
    let configured = if vi && session.posix {
        None
    } else {
        session
            .get("VISUAL")
            .filter(|s| !s.is_empty())
            .or_else(|| session.get("EDITOR").filter(|s| !s.is_empty()))
    };
    let editor = configured
        .as_deref()
        .unwrap_or(if vi { "vi" } else { "emacs" });
    if !text.ends_with('\n') {
        text.push('\n');
    }
    match crate::history::commands::edit_text(session, editor, &text, status) {
        Ok(Ok(mut edited)) => {
            if !edited.ends_with('\n') {
                edited.push('\n');
            }
            session.editor_queue = queued;
            Input::Line(edited)
        }
        Ok(Err(flow @ crate::Flow::Exit(_))) => Input::Control(flow),
        Ok(Err(_)) => Input::Cancel,
        Err(e) => {
            diagnostic!("{editor}: {e}");
            Input::Cancel
        }
    }
}

impl EditorState {
    pub(super) fn insert_last_argument(
        &mut self,
        history: &[String],
        position: usize,
        text: &mut String,
        cursor: &mut usize,
        mark: &mut usize,
    ) {
        let (end, replace) = self.last_argument.unwrap_or((position, false));
        if replace {
            self.undo.apply(text, cursor);
        }
        let next = end.checked_sub(1);
        let argument = next
            .and_then(|i| history.get(i))
            .and_then(|line| crate::parser::last_history_argument(line));
        if let Some(argument) = argument {
            *mark = if text[..*cursor].is_ascii() {
                *cursor
            } else {
                text[..*cursor].chars().count()
            };
            self.appended = (!replace && *cursor == text.len()).then_some(*cursor);
            self.undo.insert(text, cursor, argument);
        }
        self.last_argument = Some((next.unwrap_or(0), argument.is_some()));
    }

    pub(super) fn expand_history(
        &mut self,
        session: &mut Session,
        text: &mut String,
        cursor: &mut usize,
        magic_space: bool,
    ) {
        let Ok(result) = crate::history::expand::expand(session, text) else {
            return;
        };
        let at_start = *cursor == 0;
        let suffix = text.len() - *cursor;
        if let std::borrow::Cow::Owned(expanded) = result.text {
            self.undo.replace(text, 0..text.len(), &expanded, *cursor);
            *cursor = text.len();
        }
        if magic_space {
            *cursor = if at_start {
                0
            } else {
                text.len().saturating_sub(suffix)
            };
            while !text.is_char_boundary(*cursor) {
                *cursor -= 1;
            }
            self.undo.insert(text, cursor, " ");
        }
    }

    pub(super) fn accept_suggestion(
        &mut self,
        history: &mut super::History,
        session: &Session,
        view: &mut super::View,
        text: &mut String,
        cursor: &mut usize,
    ) {
        if text.is_empty() {
            return;
        }
        history.ensure(session, view, false, &|| false);
        if let Some(suffix) = history.suggestion(session.history_lines(), text, true, || true) {
            self.undo.insert(text, cursor, suffix);
        } else if let Some(suffix) = history.older_suggestion(session, view, text, &|| false) {
            self.undo.insert(text, cursor, suffix);
        }
    }

    pub(super) fn kill(
        &mut self,
        action: Action,
        text: &mut String,
        mut cursor: usize,
        mark: usize,
        killed: &mut String,
    ) -> usize {
        let (lo, hi) = match action {
            Action::Kill => (cursor, text.len()),
            Action::WholeLine => (0, text.len()),
            Action::RubFilename => {
                let sep = |c: char| c == '/' || c.is_whitespace();
                let head = text[..cursor].trim_end_matches(sep);
                (head.trim_end_matches(|c| !sep(c)).len(), cursor)
            }
            Action::KillWord => (cursor, word_end(text, cursor)),
            Action::RubWord => (
                text[..cursor]
                    .trim_end()
                    .rfind(char::is_whitespace)
                    .map_or(0, |i| next(text, i)),
                cursor,
            ),
            Action::KillBackWord => (word_start(text, cursor), cursor),
            Action::Cut | Action::Copy => {
                let point = char_offset(text, mark);
                (point.min(cursor), point.max(cursor))
            }
            _ => (0, cursor),
        };
        if action != Action::Copy {
            self.undo.record(text, lo..hi, 0, cursor, false);
        }
        if !self.killing && lo == 0 && hi == text.len() && hi > 0 && action != Action::Copy {
            std::mem::swap(killed, text);
            text.clear();
            cursor = 0;
        } else if lo != hi {
            if !self.killing { killed.clear(); }
            if self.killing && lo < cursor { killed.insert_str(0, &text[lo..hi]); }
            else { killed.push_str(&text[lo..hi]); }
            if action != Action::Copy {
                text.replace_range(lo..hi, "");
                cursor = lo;
            }
        }
        self.killing = action != Action::Copy;
        cursor
    }

    pub(super) fn change_word_case(
        &mut self,
        action: Action,
        text: &mut String,
        cursor: usize,
        locale: crate::session::CharacterLocale,
    ) -> usize {
        let end = word_end(text, cursor);
        let start = end
            - text[cursor..end]
                .trim_start_matches(|c: char| !c.is_alphanumeric())
                .len();
        if text[start..end].is_ascii() && !matches!(locale, crate::session::CharacterLocale::Turkic)
        {
            self.undo
                .record(text, start..end, end - start, cursor, false);
            let word = &mut text[start..end];
            if action == Action::UpperWord {
                word.make_ascii_uppercase();
            } else {
                word.make_ascii_lowercase();
                if action == Action::CapitalWord
                    && let Some(first) = word.get_mut(..1)
                {
                    first.make_ascii_uppercase();
                }
            }
            end
        } else {
            let word = text[start..end].to_owned();
            let replacement = if action == Action::CapitalWord {
                crate::pattern::capitalize(word, locale)
            } else {
                crate::pattern::case_chars(word, action == Action::UpperWord, true, None, locale)
            };
            self.undo.replace(text, start..end, &replacement, cursor);
            start + replacement.len()
        }
    }

    pub(super) fn transpose(&mut self, action: Action, text: &mut String, cursor: usize) -> usize {
        let (start, split, right, end) = if action == Action::TransposeWords {
            let end = word_end(text, cursor);
            let right = word_start(text, end);
            let left = word_start(text, right);
            let split = word_end(text, left);
            if left >= split || split > right || right >= end {
                return cursor;
            }
            (left, split, right, end)
        } else {
            if cursor == 0 {
                return cursor;
            }
            let end = if cursor == text.len() {
                cursor
            } else {
                next(text, cursor)
            };
            let right = previous(text, end);
            if right == 0 {
                return cursor;
            }
            (previous(text, right), right, right, end)
        };
        self.scratch.clear();
        self.scratch.push_str(&text[right..end]);
        self.scratch.push_str(&text[split..right]);
        self.scratch.push_str(&text[start..split]);
        if action == Action::TransposeWords {
            self.undo.replace(text, start..end, &self.scratch, cursor);
        } else {
            self.undo
                .record(text, start..end, end - start, cursor, false);
            text.replace_range(start..end, &self.scratch);
        }
        end
    }

    pub(super) fn execute_binding(
        &mut self,
        session: &mut Session,
        command: &str,
        status: u8,
        text: &mut String,
        cursor: &mut usize,
        mark: &mut usize,
    ) -> crate::Flow {
        let vars = session.bind(&[
            ("READLINE_LINE".into(), text.clone()),
            (
                "READLINE_POINT".into(),
                text[..*cursor].chars().count().to_string(),
            ),
            ("READLINE_MARK".into(), mark.to_string()),
        ]);
        let flow = crate::eval_text(session, command, status);
        self.undo.boundary();
        if let Some(updated) = session.get("READLINE_LINE")
            && updated != text.as_str()
        {
            let len = text.len();
            self.undo.replace(text, 0..len, &updated, *cursor);
            *cursor = text.len();
            *mark = 0;
        }
        // Readline keeps the position left by the edit for a non-decimal value.
        let offset = |name| {
            session
                .get(name)?
                .trim()
                .parse::<i64>()
                .ok()
                .map(|n| n.max(0) as usize)
        };
        if let Some(n) = offset("READLINE_POINT") {
            *cursor = char_offset(text, n);
        }
        if let Some(n) = offset("READLINE_MARK") {
            *mark = n;
        }
        session.restore(vars);
        flow
    }
}
