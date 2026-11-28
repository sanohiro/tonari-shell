// Keep the visible part of a large edit bounded even when the cursor jumps a
// megabyte. Cursor indices and history entries always refer to the full text.
use super::{Position, Screen, draw_text};
use std::fmt::Write as _;
use std::io;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub(super) struct Viewport {
    start: usize,
    image: String,
}

impl Viewport {
    pub(super) fn invalidate(&mut self) {
        self.image.clear();
    }
}

fn extent<'a>(
    clusters: impl Iterator<Item = &'a str>,
    mut budget: usize,
    width: usize,
) -> (usize, usize) {
    let mut bytes = 0;
    for cluster in clusters {
        let cells = if cluster.contains('\n') {
            width
        } else if cluster.starts_with(char::is_control) {
            2
        } else {
            cluster.width().max(1)
        };
        if cells > budget {
            break;
        }
        bytes += cluster.len();
        budget -= cells;
    }
    (bytes, budget)
}

pub(super) fn move_to(out: &mut String, from: Position, to: Position) {
    if from.row == to.row {
        if from.col > to.col {
            let _ = write!(out, "\x1b[{}D", from.col - to.col);
        }
        if from.col < to.col {
            let _ = write!(out, "\x1b[{}C", to.col - from.col);
        }
    } else {
        let (steps, direction) = if from.row > to.row {
            (from.row - to.row, 'A')
        } else {
            (to.row - from.row, 'B')
        };
        let _ = write!(out, "\x1b[{steps}{direction}\r");
        if to.col != 0 {
            let _ = write!(out, "\x1b[{}C", to.col);
        }
    }
}

impl Screen {
    // Show a partial paste without putting it into the editable line or
    // accepting any embedded newline. Copy at most a few screenfuls.
    pub(super) fn paste_frame(
        &mut self,
        prompt: &str,
        line: &str,
        cursor: usize,
        bytes: &[u8],
    ) -> io::Result<()> {
        let limit = (self.width * self.height * 4).clamp(256, 16384);
        let mut start = bytes.len().saturating_sub(limit);
        let mut view = String::new();
        if start > 0 {
            view.push('…');
            while start < bytes.len() && bytes[start] & 0xc0 == 0x80 {
                start += 1;
            }
        } else {
            let mut lo = cursor.saturating_sub(limit);
            while !line.is_char_boundary(lo) {
                lo += 1;
            }
            if lo != 0 {
                view.push('…');
            }
            view.push_str(&line[lo..cursor]);
        }
        let bytes = &bytes[start..];
        let part = match std::str::from_utf8(bytes) {
            Ok(s) => std::borrow::Cow::Borrowed(s),
            Err(e) if e.error_len().is_none() => {
                std::borrow::Cow::Borrowed(std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap())
            }
            Err(_) => String::from_utf8_lossy(bytes),
        };
        if part.contains('\r') {
            view.push_str(&part.replace("\r\n", "\n").replace('\r', "\n"));
        } else {
            view.push_str(&part);
        }
        let caret = view.len();
        let mut hi = (cursor + limit).min(line.len());
        while !line.is_char_boundary(hi) {
            hi -= 1;
        }
        view.push_str(&line[cursor..hi]);
        if hi != line.len() {
            view.push('…');
        }
        self.draw(prompt, &view, caret, "")?;
        self.tail = None;
        Ok(())
    }
    pub(super) fn window_frame(&mut self, prompt: &str, text: &str, cursor: usize, hint: &str) {
        self.output.clear();
        let mut pos = self.prompt.draw(prompt, self.width, &mut self.output);
        let width = self.width;
        let rows = self.height.saturating_sub(pos.row + 3).clamp(1, 16);
        let budget = (width * rows).saturating_sub(pos.col + 2).max(width);
        let mut start = self.view.start.min(text.len());
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        let (span, mut spare) = extent(text[start..].graphemes(true), budget, width);
        let mut end = start + span;
        if (cursor <= start && start != 0) || cursor > end || cursor == end && end < text.len() {
            let before = if cursor == text.len() {
                budget * 3 / 4
            } else {
                budget / 2
            };
            // Bindings expose character offsets, including positions inside a
            // grapheme. Measure both halves from the same caret used for drawing.
            let (prefix, unused) = extent(text[..cursor].graphemes(true).rev(), before, width);
            start = cursor - prefix;
            let (span, left) = extent(
                text[cursor..].graphemes(true),
                budget - before + unused,
                width,
            );
            end = cursor + span;
            spare = left;
        }
        self.view.start = start;
        if start != 0 {
            draw_text("…", width, &mut self.output, &mut pos);
        }
        draw_text(&text[start..cursor], width, &mut self.output, &mut pos);
        let caret = pos;
        draw_text(&text[cursor..end], width, &mut self.output, &mut pos);
        if end < text.len() {
            draw_text("…", width, &mut self.output, &mut pos);
        } else if !hint.is_empty() {
            let n = extent(hint.graphemes(true), spare, width).0;
            self.output.push_str("\x1b[2m");
            draw_text(&hint[..n], width, &mut self.output, &mut pos);
            if n < hint.len() {
                draw_text("…", width, &mut self.output, &mut pos);
            }
            self.output.push_str("\x1b[0m");
        }
        if self.output == self.view.image {
            self.output.clear();
            move_to(
                &mut self.output,
                Position {
                    row: self.row,
                    col: self.column,
                },
                caret,
            );
        } else {
            std::mem::swap(&mut self.output, &mut self.view.image);
            super::begin_redraw(&mut self.output, self.row);
            self.output.push_str(&self.view.image);
            move_to(&mut self.output, pos, caret);
        }
        self.row = caret.row;
        self.column = caret.col;
        self.had_hint = !hint.is_empty();
        self.tail = (cursor == text.len() && hint.is_empty()).then_some(caret);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_carets_render_both_halves_of_a_grapheme() {
        let text = "left e\u{301} right";
        let mut screen = Screen::new(Some((3, 8)));
        screen.window_frame("", text, 6, "");
        assert!(screen.view.image.contains("e\u{301}"));
        assert!(screen.column <= screen.width);
    }
}
