pub(crate) mod bindings;
mod editing;
mod input;
pub(crate) use input::RESIZED;
mod picker;
pub(crate) mod preview;
mod recall;
mod search;
mod undo;
mod vi;
mod viewport;

use bindings::{Action, Binding, Keymap};
pub(crate) use input::poll_readable;
use input::{Keys, Terminal, unbound_text};
use search::ReverseSearch;
use std::io::{self, Write};
use undo::Undo;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    Session, complete,
    history::{History, View},
};

pub enum Input {
    Line(String),
    End,
    Cancel,
    Directory(io::Result<()>),
    Control(crate::Flow),
}

fn matching_opener(text: &str, cursor: usize) -> Option<usize> {
    let close = *text.as_bytes().get(cursor.checked_sub(1)?)?;
    let open = match close {
        b')' => b'(',
        b']' => b'[',
        b'}' => b'{',
        _ => return None,
    };
    let mut quoted = None;
    let mut escape = false;
    let mut stack = Vec::new();
    for (i, &byte) in text.as_bytes()[..cursor].iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if byte == b'\\' && quoted != Some(b'\'') {
            escape = true;
            continue;
        }
        if let Some(quote) = quoted {
            if byte == quote {
                quoted = None;
            }
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quoted = Some(byte);
        } else if byte == open {
            stack.push(i);
        } else if byte == close {
            let found = stack.pop();
            if i + 1 == cursor {
                return found;
            }
        }
    }
    None
}

fn previous(s: &str, i: usize) -> usize {
    if i == 0 || s.as_bytes()[i - 1].is_ascii() && s.as_bytes()[i - 1] != b'\n' {
        return i.saturating_sub(1);
    }
    s[..i]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(start, _)| start)
}
fn rubout(text: &mut String, cursor: usize, overwrite: bool, undo: Option<&mut Undo>) -> usize {
    let start = previous(text, cursor);
    let blank = if overwrite && cursor < text.len() && start < cursor {
        let c = text[start..cursor].chars().next().unwrap();
        if c.is_control() {
            "  "
        } else {
            match text[start..cursor].width() {
                0 => "",
                1 => " ",
                _ => "  ",
            }
        }
    } else {
        ""
    };
    if let Some(undo) = undo {
        undo.record(text, start..cursor, blank.len(), cursor, false);
    }
    text.replace_range(start..cursor, blank);
    start
}

fn next(s: &str, i: usize) -> usize {
    let rest = &s[i..];
    if let Some(&first) = rest.as_bytes().first()
        && first.is_ascii()
        && first != b'\r'
        && rest.as_bytes().get(1).is_none_or(u8::is_ascii)
    {
        return i + 1;
    }
    i + rest.graphemes(true).next().map_or(0, str::len)
}

#[derive(Default, Clone, Copy)]
struct Position {
    row: usize,
    col: usize,
}

// Approximate byte length at which drawing switches to a window around the cursor.
const VIEWPORT_THRESHOLD: usize = 1024;

struct Screen {
    output: String,
    row: usize,
    width: usize,
    // A visible, undecorated end-of-line can accept a suffix directly.
    tail: Option<Position>,
    height: usize,
    column: usize,
    had_hint: bool,
    view: viewport::Viewport,
    prompt: PromptFrame,
}

#[derive(Default)]
struct PromptFrame {
    source: String,
    bytes: String,
    end: Position,
    width: usize,
}

impl PromptFrame {
    fn draw(&mut self, prompt: &str, width: usize, out: &mut String) -> Position {
        if prompt.len() < width && prompt.bytes().all(|b| (32..127).contains(&b)) {
            out.push_str(prompt);
            return Position {
                row: 0,
                col: prompt.len(),
            };
        }
        if self.width != width || self.source != prompt {
            self.source.clear();
            self.source.push_str(prompt);
            self.bytes.clear();
            self.end = Position::default();
            draw_prompt(prompt, width, &mut self.bytes, &mut self.end);
            self.width = width;
        }
        out.push_str(&self.bytes);
        self.end
    }
}

impl Screen {
    fn list_choices(&mut self, prompt: &str, text: &str, choices: &[String]) -> io::Result<()> {
        self.frame(prompt, text, text.len(), "");
        self.output.push_str("\r\n");
        for (i, choice) in choices.iter().enumerate() {
            if i != 0 {
                self.output.push_str("  ");
            }
            push_visible(&mut self.output, choice);
        }
        self.output.push_str("\r\n");
        io::stderr().write_all(self.output.as_bytes())?;
        self.row = 0;
        self.view.invalidate();
        Ok(())
    }

    fn new(size: Option<(u16, u16)>) -> Self {
        let (rows, columns) = size
            .or_else(|| crate::interactive::terminal_size(libc::STDERR_FILENO))
            .unwrap_or_default();
        Self {
            output: String::new(),
            row: 0,
            tail: None,
            height: if rows == 0 { 24 } else { usize::from(rows) },
            view: Default::default(),
            prompt: Default::default(),
            column: 0,
            had_hint: false,
            width: if columns < 4 {
                79
            } else {
                usize::from(columns) - 1
            },
        }
    }

    fn preview(&self) -> Self {
        let mut screen = Self::new(Some((self.height as u16, (self.width + 1) as u16)));
        screen.row = self.row;
        screen.column = self.column;
        screen.tail = self.tail;
        screen
    }

    fn resize(&mut self) {
        let Some((rows, cols)) = crate::interactive::terminal_size(libc::STDERR_FILENO) else {
            return;
        };
        if rows > 0 && cols >= 4 {
            self.width = usize::from(cols) - 1;
            self.height = usize::from(rows);
            self.view.invalidate();
            self.tail = None;
        }
    }

    fn cancel(&mut self, prompt: &str, text: &str) -> io::Result<()> {
        self.frame(prompt, text, text.len(), "");
        self.output.push_str("^C\r\n");
        io::stderr().write_all(self.output.as_bytes())
    }
    fn can_append(&self, suffix: &str) -> bool {
        suffix.len() <= self.width && !suffix.contains('\n') && self.row + 2 < self.height
    }

    fn move_short(&mut self, prompt: &str, text: &str, cursor: usize) -> io::Result<()> {
        use std::fmt::Write as _;
        self.output.clear();
        let prefix = &text[..cursor];
        let col = prompt.len() + prefix.len();
        let pos = if col < self.width
            && prompt
                .bytes()
                .chain(prefix.bytes())
                .all(|b| (32..127).contains(&b))
        {
            Position { row: 0, col }
        } else {
            let mut pos = self.prompt.draw(prompt, self.width, &mut self.output);
            draw_text(prefix, self.width, &mut self.output, &mut pos);
            self.output.clear();
            pos
        };
        if pos.row < self.row {
            let _ = write!(self.output, "\x1b[{}A", self.row - pos.row);
        }
        if pos.row > self.row {
            let _ = write!(self.output, "\x1b[{}B", pos.row - self.row);
        }
        self.output.push('\r');
        if pos.col > 0 {
            let _ = write!(self.output, "\x1b[{}C", pos.col);
        }
        self.row = pos.row;
        self.column = pos.col;
        self.tail = (cursor == text.len()).then_some(pos);
        io::stderr().write_all(self.output.as_bytes())
    }

    fn append(&mut self, suffix: &str, mut pos: Position) -> io::Result<()> {
        self.view.invalidate();
        self.output.clear();
        draw_text(suffix, self.width, &mut self.output, &mut pos);
        self.row = pos.row;
        self.column = pos.col;
        self.had_hint = false;
        self.tail = Some(pos);
        io::stderr().write_all(self.output.as_bytes())
    }

    fn draw(
        &mut self,
        prompt: &str,
        text: &str,
        cursor: usize,
        suggestion: &str,
    ) -> io::Result<()> {
        self.frame(prompt, text, cursor, suggestion);
        io::stderr().write_all(self.output.as_bytes())
    }

    fn frame(&mut self, prompt: &str, text: &str, cursor: usize, suggestion: &str) {
        if text.len().saturating_add(suggestion.len()) >= VIEWPORT_THRESHOLD
            || text.len() >= self.height
                && memchr::memchr_iter(b'\n', text.as_bytes())
                    .nth(self.height.saturating_sub(3))
                    .is_some()
            || suggestion.len() >= self.height
                && memchr::memchr_iter(b'\n', suggestion.as_bytes())
                    .nth(self.height.saturating_sub(3))
                    .is_some()
        {
            return self.window_frame(prompt, text, cursor, suggestion);
        }
        self.view.invalidate();
        let out = &mut self.output;
        begin_redraw(out, self.row);
        let mut pos = self.prompt.draw(prompt, self.width, out);
        draw_text(&text[..cursor], self.width, out, &mut pos);
        let caret = pos;
        draw_text(&text[cursor..], self.width, out, &mut pos);
        if !suggestion.is_empty() {
            out.push_str("\x1b[2m");
            draw_text(suggestion, self.width, out, &mut pos);
            out.push_str("\x1b[0m");
        }
        if pos.row != caret.row || pos.col != caret.col {
            if pos.row > caret.row {
                let _ =
                    std::fmt::Write::write_fmt(out, format_args!("\x1b[{}A", pos.row - caret.row));
            }
            out.push('\r');
            if caret.col > 0 {
                let _ = std::fmt::Write::write_fmt(out, format_args!("\x1b[{}C", caret.col));
            }
        }
        self.tail = (cursor == text.len() && suggestion.is_empty()).then_some(caret);
        self.row = caret.row;
        self.column = caret.col;
        self.had_hint = !suggestion.is_empty();
    }
}

fn control_glyph(c: char) -> [char; 2] {
    [
        '^',
        if c.is_ascii() {
            ((c as u8) ^ 64) as char
        } else {
            '?'
        },
    ]
}

fn push_visible(out: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_control() {
            out.extend(control_glyph(c));
        } else {
            out.push(c);
        }
    }
}

fn begin_redraw(out: &mut String, row: usize) {
    use std::fmt::Write;
    out.clear();
    out.push('\r');
    if row != 0 {
        let _ = write!(out, "\x1b[{row}A");
    }
    out.push_str("\x1b[J");
}

fn draw_prompt(prompt: &str, width: usize, out: &mut String, pos: &mut Position) {
    // Plain prompts fit in the current row in the common case.
    if prompt.len() <= width.saturating_sub(pos.col)
        && prompt.bytes().all(|b| (b' '..=b'~').contains(&b))
    {
        out.push_str(prompt);
        pos.col += prompt.len();
        return;
    }
    let mut escape = 0;
    let mut hidden = false;
    let mut remaining = prompt;
    while let Some(c) = remaining.chars().next() {
        if !hidden && escape == 0 && !c.is_control() {
            let end = remaining.find(char::is_control).unwrap_or(remaining.len());
            draw_text(&remaining[..end], width, out, pos);
            remaining = &remaining[end..];
            continue;
        }
        remaining = &remaining[c.len_utf8()..];
        match c {
            '\x01' => {
                hidden = true;
                continue;
            }
            '\x02' => {
                hidden = false;
                continue;
            }
            _ if hidden => {
                out.push(c);
                continue;
            }
            _ => {}
        }
        if escape != 0 || c == '\x1b' {
            out.push(c);
            escape = match (escape, c) {
                (0, _) => 1,
                (1, '[') => 2,
                (1, ']' | 'P' | 'X' | '^' | '_') => 3,
                (1 | 5, ' '..='/') => 5,
                (2, '@'..='~') | (3, '\x07') | (4, '\\') => 0,
                (3, '\x1b') => 4,
                (2, _) => 2,
                (3 | 4, _) => 3,
                _ => 0,
            };
        } else {
            draw_char(c, width, out, pos, true);
        }
    }
}

fn draw_text(mut text: &str, width: usize, out: &mut String, pos: &mut Position) {
    while !text.is_empty() {
        let mut n = text
            .bytes()
            .take_while(|b| (b' '..=b'~').contains(b))
            .count();
        if n != 0 && text.as_bytes().get(n).is_some_and(|b| !b.is_ascii()) {
            // A following combining mark or keycap can widen the final ASCII cell.
            n -= 1;
        }
        if n == 0 {
            let cluster = text.graphemes(true).next().unwrap();
            if cluster.starts_with(char::is_control) {
                for c in cluster.chars() {
                    draw_char(c, width, out, pos, false);
                }
            } else {
                let cells = cluster.width();
                if pos.col + cells > width {
                    out.push_str("\r\n");
                    pos.row += 1;
                    pos.col = 0;
                }
                out.push_str(cluster);
                pos.col += cells;
            }
            text = &text[cluster.len()..];
        } else {
            let (mut ascii, rest) = text.split_at(n);
            while !ascii.is_empty() {
                if pos.col == width {
                    out.push_str("\r\n");
                    pos.row += 1;
                    pos.col = 0;
                }
                let take = ascii.len().min(width - pos.col);
                out.push_str(&ascii[..take]);
                pos.col += take;
                ascii = &ascii[take..];
            }
            text = rest;
        }
    }
}

fn draw_char(c: char, width: usize, out: &mut String, pos: &mut Position, trusted: bool) {
    if c == '\n' {
        out.push_str("\r\n");
        pos.row += 1;
        pos.col = 0;
        return;
    }
    if c == '\r' && trusted {
        out.push('\r');
        pos.col = 0;
        return;
    }
    if c.is_control() {
        if trusted && c == '\x07' {
            out.push(c);
            return;
        }
        for ch in control_glyph(c) {
            draw_char(ch, width, out, pos, true);
        }
        return;
    }
    let cells = c.width().unwrap_or(0);
    if pos.col + cells > width {
        out.push_str("\r\n");
        pos.row += 1;
        pos.col = 0;
    }
    out.push(c);
    pos.col += cells;
}

fn char_offset(s: &str, n: usize) -> usize {
    if n >= s.len() {
        return s.len();
    }
    if s.as_bytes()[..n].is_ascii() {
        return n;
    }
    if n > s.len() / 2 {
        let distance = s.chars().count().saturating_sub(n);
        return distance
            .checked_sub(1)
            .and_then(|back| s.char_indices().rev().nth(back))
            .map_or(s.len(), |(i, _)| i);
    }
    s.char_indices().nth(n).map_or(s.len(), |(i, _)| i)
}
// An overwrite replaces complete displayed characters on both sides.
fn overwrite_end(text: &str, cursor: usize, inserted: &str, overwrite: bool) -> usize {
    if !overwrite || cursor == text.len() {
        return cursor;
    }
    let at = &text[cursor..];
    let count = if inserted.is_ascii() && !inserted.as_bytes().contains(&b'\r') {
        inserted.len()
    } else {
        inserted.graphemes(true).count()
    };
    if let Some(prefix) = at.as_bytes().get(..=count)
        && prefix.is_ascii()
        && !prefix.contains(&b'\r')
    {
        return cursor + count;
    }
    cursor
        + at.grapheme_indices(true)
            .nth(count)
            .map_or(at.len(), |(i, _)| i)
}

fn word_start(s: &str, mut i: usize) -> usize {
    for alnum in [false, true] {
        while i > 0 {
            let j = previous(s, i);
            if s[j..i].starts_with(char::is_alphanumeric) != alnum {
                break;
            }
            i = j;
        }
    }
    i
}
fn word_end(s: &str, mut i: usize) -> usize {
    for alnum in [false, true] {
        while i < s.len() {
            if s[i..].starts_with(char::is_alphanumeric) != alnum {
                break;
            }
            i = next(s, i);
        }
    }
    i
}
#[derive(Clone, Default)]
pub(crate) struct KillRing {
    entries: std::collections::VecDeque<String>,
}
impl KillRing {
    fn entry(&mut self, continuing: bool) -> &mut String {
        if self.entries.is_empty() || !continuing {
            const MAX_ENTRIES: usize = 16;
            const MAX_RETAINED_BYTES: usize = 4 * 1024 * 1024;
            let mut bytes: usize = self.entries.iter().map(String::len).sum();
            while self.entries.len() >= MAX_ENTRIES || bytes > MAX_RETAINED_BYTES {
                bytes -= self.entries.pop_back().unwrap().len();
            }
            self.entries.push_front(String::new());
        }
        self.entries.front_mut().unwrap()
    }
}

pub enum Start<'a> {
    Command(String),
    Read { initial: &'a str, files_only: bool },
}

#[derive(Clone, Copy)]
struct LineView<'a> {
    prompt: &'a str,
    text: &'a str,
    cursor: usize,
}

#[derive(Default)]
struct EditorState {
    undo: Undo,
    scratch: String,
    overwrite: bool,
    killing: bool,
    yank: Option<(usize, usize)>,
    motion: bool,
    appended: Option<usize>,
    erased: Option<usize>,
    last_argument: Option<(usize, bool)>,
}

impl EditorState {
    fn insert(
        &mut self,
        keys: &mut Keys,
        first: u8,
        session: &Session,
        text: &mut String,
        cursor: &mut usize,
    ) -> io::Result<Option<usize>> {
        if *cursor == text.len() {
            let start = *cursor;
            keys.append_burst(first, session, text)?;
            self.undo
                .record(text, start..start, text.len() - start, start, true);
            *cursor = text.len();
            return Ok(Some(start));
        }
        self.scratch.clear();
        keys.append_burst(first, session, &mut self.scratch)?;
        let inserted = &self.scratch;
        let end = overwrite_end(text, *cursor, inserted, self.overwrite);
        self.undo
            .record(text, *cursor..end, inserted.len(), *cursor, true);
        text.replace_range(*cursor..end, inserted);
        *cursor += inserted.len();
        Ok(None)
    }

    fn render_frame(
        &mut self,
        screen: &mut Screen,
        line: LineView<'_>,
        hint: &str,
        searching: bool,
        replaying: bool,
    ) -> io::Result<()> {
        let LineView {
            prompt,
            text,
            cursor,
        } = line;
        let moving = std::mem::take(&mut self.motion);
        if replaying {
            self.appended = None;
            self.erased = None;
        } else if moving
            && !screen.had_hint
            && hint.is_empty()
            && !searching
            && text.len() < VIEWPORT_THRESHOLD
            && !text.contains('\n')
        {
            screen.move_short(prompt, text, cursor)?;
        } else if let Some(start) = self.appended.take()
            && cursor == text.len()
            && !searching
            && screen.can_append(&text[start..])
            && (start == 0
                || text[start..].is_ascii()
                || previous(text, next(text, start)) == start)
            && (!screen.had_hint || text.len().saturating_add(hint.len()) < VIEWPORT_THRESHOLD)
            && (screen.had_hint || screen.tail.is_some() && hint.is_empty())
        {
            if screen.had_hint {
                screen.output.clear();
                screen.output.push_str("\x1b[J");
                let mut caret = Position {
                    row: screen.row,
                    col: screen.column,
                };
                draw_text(&text[start..], screen.width, &mut screen.output, &mut caret);
                if !hint.is_empty() {
                    screen.output.push_str("\x1b[2m");
                    let mut end = caret;
                    draw_text(hint, screen.width, &mut screen.output, &mut end);
                    screen.output.push_str("\x1b[0m");
                    viewport::move_to(&mut screen.output, end, caret);
                }
                screen.tail = hint.is_empty().then_some(caret);
                screen.had_hint = !hint.is_empty();
                screen.row = caret.row;
                screen.column = caret.col;
                screen.view.invalidate();
                io::stderr().write_all(screen.output.as_bytes())?;
            } else if let Some(pos) = screen.tail {
                screen.append(&text[start..], pos)?;
            }
        } else if let Some(cells) = self.erased.take()
            && let Some(mut pos) = screen.tail
            && cells < pos.col
            && cursor == text.len()
            && hint.is_empty()
            && !searching
        {
            pos.col -= cells;
            screen.tail = Some(pos);
            screen.column = pos.col;
            screen.view.invalidate();
            screen.output.clear();
            let _ =
                std::fmt::Write::write_fmt(&mut screen.output, format_args!("\x1b[{cells}D\x1b[K"));
            io::stderr().write_all(screen.output.as_bytes())?;
        } else {
            screen.draw(prompt, text, cursor, hint)?;
        }
        Ok(())
    }
}

#[cold]
fn read_trap(
    term: &mut Option<Terminal>,
    session: &mut Session,
    status: u8,
    screen: &mut Screen,
    recall: &mut recall::Recall,
) -> io::Result<Option<Input>> {
    // A handler may run another terminal program, including a nested read.
    drop(term.take());
    io::stderr().write_all(b"\r\n")?;
    let flow = {
        let _deadline = crate::input_deadline(None);
        crate::traps::dispatch(session, status)
    };
    if !matches!(flow, crate::Flow::Continue(_)) {
        return Ok(Some(Input::Control(flow)));
    }
    // A handler may clear or replace the history table while editing.
    *recall = recall::Recall::new(session.history_lines().len());
    *term = Some(Terminal::enter(session.bindings.paste())?);
    screen.row = 0;
    screen.resize();
    screen.tail = None;
    screen.view.invalidate();
    Ok(None)
}

pub fn line(
    prompt: &str,
    history: &mut History,
    session: &mut Session,
    status: u8,
    start: Start<'_>,
    primary_prompt: bool,
    terminal_size: Option<(u16, u16)>,
) -> io::Result<Input> {
    bindings::initialize(session);
    session.bindings.map = if session.vi {
        Keymap::Insert
    } else {
        Keymap::Emacs
    };
    let mut term = Some(Terminal::enter(session.bindings.paste())?);
    let mut keys = Keys::new(std::mem::take(&mut session.editor_queue));
    let mut edit = EditorState::default();
    let mut mark = 0usize;
    let mut screen = Screen::new(terminal_size);
    let (mut text, read) = match start {
        Start::Command(initial) => (initial, None),
        Start::Read {
            initial,
            files_only,
        } => (initial.to_owned(), Some(files_only)),
    };
    let mut cursor = text.len();
    let mut vi = vi::State::default();
    let mut recall = recall::Recall::new(session.history_lines().len());
    let mut search: Option<ReverseSearch> = None;
    let mut search_prompt = String::new();
    let mut last_tab = false;
    let mut blink = false;
    let mut replaying = false;
    let mut view = View::Unread;
    if read.is_none() && keys.queued.is_empty() {
        history.prepare_search(session, &|| poll_readable(0, 0));
    }
    // A signal can interrupt any part of one input operation. Retry that
    // operation with its consumed bytes intact after running a read handler.
    macro_rules! read_key {
        ($operation:expr) => {
            loop {
                match $operation {
                    Ok(value) => break value,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                        if RESIZED.swap(false, std::sync::atomic::Ordering::Relaxed) {
                            screen.resize();
                            screen.draw(prompt, &text, cursor, "")?;
                        } else if session.traps.read_pending() {
                            if let Some(outcome) =
                                read_trap(&mut term, session, status, &mut screen, &mut recall)?
                            {
                                session.editor_queue = std::mem::take(&mut keys.queued);
                                return Ok(outcome);
                            }
                        } else {
                            screen.cancel(prompt, &text)?;
                            return Ok(Input::Cancel);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        };
    }
    loop {
        if session.traps.read_pending()
            && let Some(outcome) = read_trap(&mut term, session, status, &mut screen, &mut recall)?
        {
            session.editor_queue = std::mem::take(&mut keys.queued);
            return Ok(outcome);
        }
        if RESIZED.swap(false, std::sync::atomic::Ordering::Relaxed) {
            screen.resize();
        }
        if session.bindings.map == Keymap::Command && !text.is_empty() {
            cursor = cursor.min(previous(&text, text.len()));
        }
        // Apply already visible typeahead to the completion before repainting.
        // A custom binding or control key must see the settled line first.
        replaying = replaying && keys.can_replay(session);
        let show_hint = !replaying
            && read.is_none()
            && search.is_none()
            && cursor == text.len()
            && !text.is_empty();
        let hint = if show_hint {
            history
                .suggestion(session.history_lines(), &text, view != View::Unread, || {
                    false
                })
                .unwrap_or_default()
        } else {
            ""
        };
        let visible_prompt = if let Some(search) = &search {
            search.prompt(screen.width, &mut search_prompt);
            &search_prompt
        } else {
            prompt
        };
        let needs_hint = hint.is_empty();
        edit.render_frame(
            &mut screen,
            LineView {
                prompt: visible_prompt,
                text: &text,
                cursor,
            },
            hint,
            search.is_some(),
            replaying,
        )?;
        if std::mem::take(&mut blink)
            && keys.queued.is_empty()
            && !poll_readable(0, 0)
            && let Some(open) = matching_opener(&text, cursor)
        {
            screen.draw(visible_prompt, &text, open, hint)?;
            poll_readable(0, 500);
            screen.draw(visible_prompt, &text, cursor, hint)?;
        }
        if let Some(search) = &mut search
            && let Some(found) =
                search.update(&text, history, session, &mut view, &|| keys.input_pending())
        {
            if !search.query.is_empty() {
                history.last_search.clone_from(&search.query);
            }
            text = found;
            cursor = text.len();
            screen.draw(visible_prompt, &text, cursor, "")?;
        }
        if show_hint {
            let input_ready = || keys.input_pending();
            // After showing a recalled command, let the terminal paint it or send
            // the next arrow before searching older rows for a longer suggestion.
            if needs_hint
                && !(recall.on_match(session.history_lines().len())
                    && (!keys.queued.is_empty() || poll_readable(0, 1)))
                && history.ensure(session, &mut view, false, &input_ready)
            {
                if let Some(hint) =
                    history.suggestion(session.history_lines(), &text, true, || !input_ready())
                {
                    screen.draw(visible_prompt, &text, cursor, hint)?;
                } else if let Some(hint) =
                    history.older_suggestion(session, &mut view, &text, &input_ready)
                {
                    screen.draw(visible_prompt, &text, cursor, hint)?;
                }
            }
        }
        if crate::interrupted() {
            screen.cancel(prompt, &text)?;
            return Ok(Input::Cancel);
        }
        if session.notify && keys.queued.is_empty() {
            crate::wait_for_job_notice(&mut session.jobs, |jobs| {
                screen.draw("", "", 0, "")?;
                jobs.report_changes(session.posix);
                screen.draw(visible_prompt, &text, cursor, "")
            })?;
        }
        let Some(first) = read_key!(keys.get()) else {
            session.editor_queue = std::mem::take(&mut keys.queued);
            return Ok(Input::End);
        };
        // Ordinary committed text can bypass key-sequence and editing-action dispatch.
        if search.is_none() && unbound_text(first, session) {
            edit.killing = false;
            edit.yank = None;
            edit.last_argument = None;
            recall.edited(session.history_lines().len());
            let start = cursor;
            edit.appended =
                read_key!(edit.insert(&mut keys, first, session, &mut text, &mut cursor));
            blink = session.bindings.blink_matching_paren
                && matches!(&text.as_bytes()[start..cursor], b")" | b"]" | b"}");
            last_tab = false;
            continue;
        }
        let (binding, right_arrow) = read_key!(keys.binding(first, session));
        let action = match binding {
            Binding::Removed if first == 27 && search.is_some() => Action::End,
            Binding::Removed => {
                vi = vi::State::default();
                continue;
            }
            Binding::Macro(seq) => {
                keys.expand_macro(&seq)?;
                continue;
            }
            Binding::Shell(command) => {
                drop(term);
                io::stderr().write_all(b"\r\n")?;
                let flow = edit.execute_binding(
                    session,
                    &command,
                    status,
                    &mut text,
                    &mut cursor,
                    &mut mark,
                );
                edit.last_argument = None;
                edit.killing = false;
                edit.yank = None;
                recall = recall::Recall::new(session.history_lines().len());
                if let crate::Flow::Exit(_) = flow {
                    session.editor_queue = std::mem::take(&mut keys.queued);
                    return Ok(Input::Control(flow));
                }
                term = Some(Terminal::enter(session.bindings.paste())?);
                screen.row = 0;
                screen.view.invalidate();
                continue;
            }
            Binding::Edit(a) => a,
        };
        if !matches!(action, Action::Yank | Action::YankPop) {
            edit.yank = None;
        }
        if action != Action::LastArg {
            edit.last_argument = None;
        }
        if !matches!(
            action,
            Action::Kill
                | Action::WholeLine
                | Action::KillBack
                | Action::KillWord
                | Action::RubWord
                | Action::KillBackWord
                | Action::RubFilename
                | Action::Cut
                | Action::Discard
        ) {
            edit.killing = false;
        }
        if action != Action::Insert {
            edit.undo.boundary();
        }
        let history_position = recall.position();
        if !matches!(
            action,
            Action::Up
                | Action::Down
                | Action::FirstHistory
                | Action::LastHistory
                | Action::PrefixUp
                | Action::PrefixDown
        ) {
            recall.edited(session.history_lines().len());
        }
        let action = match action {
            Action::PreviousDirectory | Action::NextDirectory
                if primary_prompt && text.is_empty() && search.is_none() =>
            {
                let result = session.revisit_directory(action == Action::NextDirectory);
                if !matches!(result, Ok(false)) {
                    screen.draw("", "", 0, "")?;
                    session.editor_queue = std::mem::take(&mut keys.queued);
                    return Ok(Input::Directory(result.map(|_| ())));
                }
                continue;
            }
            Action::PreviousDirectory => Action::BackWord,
            Action::NextDirectory => Action::ForwardWord,
            action => action,
        };
        if action == Action::Search && search.is_none() {
            search = Some(ReverseSearch::new(&text, cursor));
            // Readline keeps the current draft until a nonempty query matches.
            continue;
        }
        if let Some(state) = search.as_mut() {
            match action {
                Action::Abort => {
                    text = std::mem::take(&mut state.original);
                    cursor = state.original_cursor;
                    search = None;
                    continue;
                }
                Action::Accept | Action::Forward | Action::Back | Action::Home | Action::End => {
                    if !state.pending.is_empty()
                        && let Ok(Some(found)) =
                            history.find_command(session, &mut view, &state.pending, &|| false)
                    {
                        if !state.query.is_empty() {
                            history.last_search.clone_from(&state.query);
                        }
                        text = found;
                    }
                    if text != state.original {
                        edit.undo.record(
                            &state.original,
                            0..state.original.len(),
                            text.len(),
                            state.original_cursor,
                            false,
                        );
                    }
                    search = None;
                    cursor = text.len();
                }
                _ => {
                    if action == Action::Search
                        && state.query.is_empty()
                        && !history.last_search.is_empty()
                    {
                        state.query.clone_from(&history.last_search);
                        state.queue(None);
                        continue;
                    }
                    let mut partial = Vec::new();
                    let shortest = read_key!(state.edit_query(
                        action,
                        first,
                        &mut keys,
                        session,
                        &mut partial,
                        |bytes| screen.paste_frame(visible_prompt, "", 0, bytes)
                    ));
                    if state.wants_picker(action) {
                        let selected = read_key!(picker::history(
                            &mut state.query,
                            history,
                            session,
                            &mut keys,
                            &mut screen,
                        ));
                        match selected {
                            Some(command) => {
                                edit.undo = Undo::default();
                                text = command;
                                cursor = text.len();
                            }
                            None => {
                                text = std::mem::take(&mut state.original);
                                cursor = state.original_cursor;
                            }
                        }
                        search = None;
                        last_tab = false;
                        continue;
                    }
                    state.queue(shortest);
                    cursor = text.len();
                    continue;
                }
            }
        }
        if session.bindings.map == Keymap::Command
            && let Some(moved) = vi.apply(
                action,
                first,
                &mut text,
                &mut cursor,
                session,
                &mut edit.undo,
            )
        {
            edit.motion = moved;
            last_tab = false;
            continue;
        }
        edit.motion = matches!(
            action,
            Action::Back
                | Action::BackWord
                | Action::ForwardWord
                | Action::Home
                | Action::End
                | Action::SwapMark
        ) || action == Action::Forward && (cursor < text.len() || !right_arrow);
        let was_tab = last_tab;
        last_tab = action == Action::Complete;
        match action {
            Action::Redraw => {
                screen.view.invalidate();
            }
            Action::Overwrite => edit.overwrite = !edit.overwrite,
            Action::Vi
            | Action::ViInsert
            | Action::ViAppend
            | Action::ViAppendEnd
            | Action::ViFirst => {
                session.vi = true;
                session.emacs = false;
                session.bindings.map = Keymap::Insert;
                cursor = match action {
                    Action::ViAppend => next(&text, cursor),
                    Action::ViAppendEnd => text.len(),
                    Action::ViFirst => text.len() - text.trim_start().len(),
                    _ => cursor,
                };
            }
            Action::EditAndExecute | Action::ViEditAndExecute => {
                drop(term);
                io::stderr().write_all(b"\r\n")?;
                return Ok(editing::edit_and_execute(
                    session,
                    text,
                    action == Action::ViEditAndExecute,
                    status,
                    keys.queued,
                ));
            }
            Action::ViCommand => {
                session.bindings.map = Keymap::Command;
                cursor = previous(&text, cursor);
            }
            Action::Emacs => {
                session.vi = false;
                session.emacs = true;
                session.bindings.map = Keymap::Emacs;
            }
            Action::Abort => {
                io::stderr().write_all(b"\x07")?;
            }
            Action::Delete if text.is_empty() && first == 4 => {
                io::stderr().write_all(b"\r\n")?;
                session.editor_queue = std::mem::take(&mut keys.queued);
                return Ok(Input::End);
            }
            Action::Accept | Action::Comment => {
                if action == Action::Comment {
                    text.insert_str(0, session.bindings.comment_begin.as_deref().unwrap_or("#"));
                }
                if action == Action::Comment
                    || visible_prompt != prompt
                    || replaying
                    || screen.tail.is_none()
                {
                    screen.frame(prompt, &text, text.len(), "");
                } else {
                    screen.output.clear();
                }
                screen.output.push_str("\r\n");
                io::stderr().write_all(screen.output.as_bytes())?;
                text.push('\n');
                session.editor_queue = std::mem::take(&mut keys.queued);
                return Ok(Input::Line(text));
            }
            Action::Home => cursor = 0,
            Action::End => cursor = text.len(),
            Action::Back => cursor = previous(&text, cursor),
            Action::Forward if !right_arrow => cursor = next(&text, cursor),
            Action::Delete => {
                let end = next(&text, cursor);
                edit.undo.record(&text, cursor..end, 0, cursor, false);
                text.replace_range(cursor..end, "");
            }
            Action::Rubout => {
                let start = previous(&text, cursor);
                if cursor == text.len()
                    && cursor - start == 1
                    && text.as_bytes()[start].is_ascii_graphic()
                {
                    edit.erased = Some(1usize);
                }
                cursor = rubout(&mut text, cursor, edit.overwrite, Some(&mut edit.undo));
            }
            Action::Kill
            | Action::Discard
            | Action::KillBack
            | Action::RubWord
            | Action::KillBackWord
            | Action::WholeLine
            | Action::RubFilename
            | Action::KillWord
            | Action::Cut
            | Action::Copy => {
                cursor = edit.kill(action, &mut text, cursor, mark, &mut session.editor_kill);
            }
            Action::Undo => {
                edit.undo.apply(&mut text, &mut cursor);
                screen.view.invalidate();
            }
            Action::LastArg => edit.insert_last_argument(
                session.history_lines(),
                history_position,
                &mut text,
                &mut cursor,
                &mut mark,
            ),
            Action::Yank => {
                if let Some(value) = session.editor_kill.entries.front() {
                    edit.yank = Some((cursor, 0));
                    edit.undo.insert(&mut text, &mut cursor, value);
                }
            }
            Action::YankPop => {
                if let Some((start, index)) = edit.yank
                    && !session.editor_kill.entries.is_empty()
                {
                    let next = (index + 1) % session.editor_kill.entries.len();
                    let value = &session.editor_kill.entries[next];
                    edit.undo.replace(&mut text, start..cursor, value, cursor);
                    cursor = start + value.len();
                    edit.yank = Some((start, next));
                }
            }
            Action::Mark => mark = text[..cursor].chars().count(),
            Action::SwapMark => {
                let p = char_offset(&text, mark);
                mark = text[..cursor].chars().count();
                cursor = p;
            }
            Action::BackWord => cursor = word_start(&text, cursor),
            Action::ForwardWord if cursor == text.len() && screen.had_hint => {
                if let Some(line) = history.displayed(session.history_lines())
                    && line.starts_with(&text)
                {
                    let end = crate::parser::suggestion_end(line, cursor);
                    let word = &line[cursor..end];
                    edit.undo.insert(&mut text, &mut cursor, word);
                }
            }
            Action::ForwardWord => cursor = word_end(&text, cursor),
            Action::Reload => bindings::reload(session),
            Action::TabInsert => {
                edit.undo.insert(&mut text, &mut cursor, "\t");
            }
            Action::HistoryExpand | Action::MagicSpace => {
                edit.expand_history(
                    session,
                    &mut text,
                    &mut cursor,
                    action == Action::MagicSpace,
                );
            }
            Action::UpperWord | Action::LowerWord | Action::CapitalWord => {
                cursor =
                    edit.change_word_case(action, &mut text, cursor, session.character_locale());
            }
            Action::TransposeWords | Action::Transpose => {
                cursor = edit.transpose(action, &mut text, cursor);
            }
            Action::Quote => {
                if let Some(key) = read_key!(keys.get())
                    && let Some(c) = read_key!(keys.character(key))
                {
                    edit.undo
                        .insert(&mut text, &mut cursor, c.encode_utf8(&mut [0; 4]));
                }
            }
            Action::Clear => {
                io::stderr().write_all(b"\x1b[2J\x1b[H")?;
                screen.row = 0;
                screen.view.invalidate();
            }
            Action::Complete | Action::List if !session.bindings.disable_completion => {
                let was_tab = was_tab || action == Action::List;
                let bindings = session.bindings.clone();
                let result = preview::during(
                    LineView {
                        prompt,
                        text: &text,
                        cursor,
                    },
                    &mut screen,
                    &mut keys,
                    bindings,
                    edit.overwrite,
                    || complete::at(&text, cursor, session, was_tab, read.unwrap_or(false)),
                );
                replaying = !keys.queued.is_empty();
                screen.tail = None;
                screen.view.invalidate();
                if let Some(result) = result {
                    if was_tab && result.choices.len() > 1 && action == Action::Complete {
                        last_tab = false;
                        let selected = read_key!(picker::choose(
                            &result.choices,
                            &mut keys,
                            &mut screen,
                            session
                        ));
                        replaying = false;
                        if let Some(index) = selected {
                            let word = result.select(index);
                            edit.undo
                                .replace(&mut text, result.start..cursor, &word, cursor);
                            cursor = result.start + word.len();
                        }
                    } else if was_tab && result.choices.len() > 1 {
                        screen.list_choices(prompt, &text, &result.choices)?;
                    } else {
                        edit.undo.replace(
                            &mut text,
                            result.start..cursor,
                            &result.replacement,
                            cursor,
                        );
                        cursor = result.start + result.replacement.len();
                    }
                }
            }
            Action::PrefixUp
            | Action::PrefixDown
            | Action::Up
            | Action::Down
            | Action::FirstHistory
            | Action::LastHistory => {
                recall.navigate(
                    action,
                    session.history_lines(),
                    &mut text,
                    &mut cursor,
                    keys.anchor_history,
                );
                if recall.position() != history_position {
                    edit.undo = Undo::default();
                }
            }
            Action::PreviousDirectory | Action::NextDirectory => unreachable!(),
            Action::Forward if cursor == text.len() => {
                edit.accept_suggestion(history, session, &mut view, &mut text, &mut cursor);
            }
            Action::Forward => cursor = next(&text, cursor),
            Action::Paste => {
                if cursor == text.len() {
                    edit.appended = Some(cursor);
                }
                let mut partial = Vec::new();
                let pasted = read_key!(keys.resume_paste(&mut partial, |bytes| {
                    screen.paste_frame(prompt, &text, cursor, bytes)
                }));
                edit.undo
                    .record(&text, cursor..cursor, pasted.len(), cursor, false);
                let end = cursor + pasted.len();
                if text.is_empty() {
                    text = pasted;
                } else {
                    text.insert_str(cursor, &pasted);
                }
                cursor = end;
            }
            Action::Insert => {
                edit.appended =
                    read_key!(edit.insert(&mut keys, first, session, &mut text, &mut cursor));
            }
            _ => {}
        }
        if edit.motion && matches!(action, Action::Back | Action::Forward) && search.is_none() {
            cursor = keys.motion_burst(&text, cursor, session);
        }
    }
}
