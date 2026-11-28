// Keep the visible part of a large edit bounded even when the cursor jumps a
// megabyte. Cursor indices and history entries always refer to the full text.
use super::{Position, Screen, draw_prompt, draw_text};
use std::fmt::Write as _;
use std::io;
use unicode_width::UnicodeWidthChar;

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

fn cost(c: char, width: usize) -> usize {
    match c {
        '\n' => width,
        c if c.is_control() => 2,
        c => c.width().unwrap_or(0).max(1),
    }
}

fn extent(chars: impl Iterator<Item = char>, mut budget: usize, width: usize) -> (usize, usize) {
    let mut bytes = 0;
    for c in chars {
        let cells = cost(c, width);
        if cells > budget {
            break;
        }
        bytes += c.len_utf8();
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
        let mut pos = Position::default();
        draw_prompt(prompt, self.width, &mut self.output, &mut pos);
        let width = self.width;
        let rows = self.height.saturating_sub(pos.row + 3).clamp(1, 16);
        let budget = (width * rows).saturating_sub(pos.col + 2).max(width);
        let mut start = self.view.start.min(text.len());
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        let (span, mut spare) = extent(text[start..].chars(), budget, width);
        let mut end = start + span;
        if (cursor <= start && start != 0) || cursor > end || cursor == end && end < text.len() {
            let before = if cursor == text.len() {
                budget * 3 / 4
            } else {
                budget / 2
            };
            start = cursor - extent(text[..cursor].chars().rev(), before, width).0;
            let (span, left) = extent(text[start..].chars(), budget, width);
            end = start + span;
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
            let n = extent(hint.chars(), spare, width).0;
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
