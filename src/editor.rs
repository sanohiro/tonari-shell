pub(crate) mod bindings;
mod editing;
mod picker;
pub(crate) mod preview;
mod recall;
mod undo;
use undo::Undo;
mod vi;
mod viewport;
use bindings::{Action, Binding, Keymap};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::os::fd::RawFd;
use unicode_width::UnicodeWidthChar;

use crate::{
    Session, complete,
    history::{History, SearchQuery, View},
};

pub enum Input {
    Line(String),
    End,
    Cancel,
    Directory(io::Result<()>),
    Exit(u8),
}

struct Terminal(libc::termios, bool);

impl Terminal {
    fn enter(bracketed: bool) -> io::Result<Self> {
        let mut old = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(0, &mut old) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut new = old;
        new.c_lflag &= !(libc::ICANON | libc::ECHO | libc::IEXTEN);
        new.c_iflag &= !(libc::IXON | libc::ICRNL);
        new.c_cc[libc::VSUSP] = libc::_POSIX_VDISABLE as _;
        new.c_cc[libc::VMIN] = 0;
        new.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &new) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let term = Self(old, bracketed);
        if bracketed {
            io::stderr().write_all(b"\x1b[?2004h")?;
        }
        Ok(term)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        if self.1 {
            let _ = io::stderr().write_all(b"\x1b[?2004l");
        }
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.0);
        }
    }
}

fn byte() -> io::Result<Option<u8>> {
    let mut key = [0u8];
    loop {
        match crate::terminal_read(&mut key, true) {
            Ok(n) => return Ok((n == 1).then_some(key[0])),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e),
        }
    }
}

pub(crate) fn ready(fd: RawFd, ms: i32) -> bool {
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut p, 1, ms) > 0 }
}

#[derive(Default)]
struct Keys {
    queued: VecDeque<u8>,
    expansions: usize,
}

impl Keys {
    fn input_pending(&self) -> bool {
        !self.queued.is_empty()
            || ready(0, 0)
            || crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn get(&mut self) -> io::Result<Option<u8>> {
        if let Some(key) = self.queued.pop_front() {
            return Ok(Some(key));
        }
        self.expansions = 0;
        byte()
    }

    // VMIN and VTIME are zero. Only consume bytes belonging to this edit:
    // a foreground child can read the terminal immediately after accept-line.
    fn immediate(&mut self) -> Option<u8> {
        if let Some(b) = self.queued.pop_front() {
            return Some(b);
        }
        let mut b = 0u8;
        if unsafe { libc::read(0, std::ptr::addr_of_mut!(b).cast(), 1) } == 1 {
            self.expansions = 0;
            Some(b)
        } else {
            None
        }
    }

    // A terminal repeat or macro may deliver several completed motions at once.
    // Stop before a partial escape, custom binding, or suggestion-accepting Right.
    fn motion_burst(&mut self, text: &str, mut cursor: usize, session: &Session) -> usize {
        if session.bindings.map != Keymap::Emacs {
            return cursor;
        }
        for _ in 0..64 {
            let Some(first) = self.immediate() else { break };
            let mut seq = [first, 0, 0];
            let mut len = 1;
            if first == 27
                && !session.bindings.custom_prefix(&[first])
                && let Some(b) = self.immediate()
            {
                seq[1] = b;
                len = 2;
                if matches!(b, b'[' | b'O')
                    && let Some(b) = self.immediate()
                {
                    seq[2] = b;
                    len = 3;
                }
            }
            match &seq[..len] {
                [2] if !session.bindings.custom_prefix(&[2]) => cursor = previous(text, cursor),
                [6] if !session.bindings.custom_prefix(&[6]) => cursor = next(text, cursor),
                [27, b'[' | b'O', b'D'] => cursor = previous(text, cursor),
                [27, b'[' | b'O', b'C'] if cursor < text.len() => cursor = next(text, cursor),
                other => {
                    for &b in other.iter().rev() {
                        self.queued.push_front(b);
                    }
                    break;
                }
            }
        }
        cursor
    }

    fn character(&mut self, first: u8) -> io::Result<Option<char>> {
        if first.is_ascii() {
            return Ok(Some(char::from(first)));
        }
        let Some(width) = multibyte_width(first) else {
            return Ok(None);
        };
        let mut bytes = [first; 4];
        for slot in &mut bytes[1..width] {
            let b = if let Some(b) = self.immediate() {
                b
            } else if let Some(b) = self.get()? {
                b
            } else {
                return Ok(None);
            };
            if b & 0xc0 != 0x80 {
                self.queued.push_front(b);
                return Ok(None);
            }
            *slot = b;
        }
        Ok(std::str::from_utf8(&bytes[..width])
            .ok()
            .and_then(|s| s.chars().next()))
    }

    fn insert_burst(&mut self, first: u8, session: &Session, out: &mut String) -> io::Result<()> {
        out.clear();
        let Some(c) = self.character(first)? else {
            return Ok(());
        };
        out.push(c);
        if session.bindings.map == Keymap::Command {
            return Ok(());
        }
        // Gather an immediately available run without waiting for more keys.
        // A bound keeps long streams responsive to signals and screen updates.
        let mut bytes = [0; 4096];
        let mut n = 0;
        while n < bytes.len() {
            let Some(b) = self.immediate() else { break };
            if b < 32 || b == 127 || session.bindings.custom_prefix(&[b]) {
                self.queued.push_front(b);
                break;
            }
            bytes[n] = b;
            n += 1;
        }
        let mut rest = &bytes[..n];
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    out.push_str(text);
                    break;
                }
                Err(e) => {
                    let (valid, tail) = rest.split_at(e.valid_up_to());
                    out.push_str(std::str::from_utf8(valid).unwrap());
                    if let Some(invalid) = e.error_len() {
                        rest = &tail[invalid..];
                    } else {
                        // Display the valid prefix before waiting for a split scalar.
                        for &b in tail.iter().rev() {
                            self.queued.push_front(b);
                        }
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn binding(&mut self, first: u8, session: &Session) -> io::Result<(Binding, bool)> {
        if unbound_text(first, session) {
            return Ok((Binding::Edit(Action::Insert), false));
        }
        let fallback = if session.bindings.map != Keymap::Command && first >= 32 && first != 127 {
            Binding::Edit(Action::Insert)
        } else {
            Binding::Removed
        };
        let (exact, mut longer) = session.bindings.lookup(&[first]);
        if !longer {
            return Ok((exact.unwrap_or(fallback), false));
        }
        let mut seq = vec![first];
        let mut best = exact.map(|action| (1, action));
        while longer {
            // Consume one byte at a time; a foreground child owns later typeahead.
            let key = if let Some(key) = self.immediate() {
                key
            } else if ready(0, session.bindings.timeout()) {
                let Some(key) = self.get()? else { break };
                key
            } else {
                break;
            };
            seq.push(key);
            let (exact, prefix) = session.bindings.lookup(&seq);
            if let Some(action) = exact {
                best = Some((seq.len(), action));
            }
            longer = prefix;
        }
        let (len, action) = best.unwrap_or((1, fallback));
        // Preserve unmatched bytes when a custom sequence shares a prefix with ordinary typing.
        for &key in seq[len..].iter().rev() {
            self.queued.push_front(key);
        }
        Ok((
            action,
            len > 1 && matches!(&seq[..len], b"\x1b[C" | b"\x1bOC"),
        ))
    }

    // After the end marker one byte still belongs to the editor. It could be
    // Enter, so leave everything after that byte on the tty for the next reader.
    fn paste(&mut self, mut paint: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<String> {
        const END: &[u8; 6] = b"\x1b[201~";
        let mut data = Vec::new();
        let mut matched = 0;
        let mut chunk = [0; END.len() + 1];
        let mut displayed = 0;
        let mut checkpoint = 16 * 1024;
        let mut painted_at = std::time::Instant::now();
        'paste: loop {
            if crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
                || crate::traps::input_interrupted()
            {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let available = data.len() - matched;
            if available >= checkpoint {
                checkpoint = available + 16 * 1024;
                if painted_at.elapsed() >= std::time::Duration::from_millis(16) {
                    paint(&data[..available])?;
                    displayed = available;
                    painted_at = std::time::Instant::now();
                }
            }
            let limit = chunk.len() - matched;
            let n = if self.queued.is_empty() {
                let immediate = unsafe { libc::read(0, chunk.as_mut_ptr().cast(), limit) };
                if immediate > 0 {
                    immediate as usize
                } else {
                    // A split packet may pause for an arbitrary time. Give a
                    // contiguous paste one short frame interval to resume.
                    if available > displayed && !ready(0, 8) {
                        paint(&data[..available])?;
                        displayed = available;
                        painted_at = std::time::Instant::now();
                    }
                    match crate::terminal_read(&mut chunk[..limit], true) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                        Err(e) => return Err(e),
                    }
                }
            } else {
                let n = limit.min(self.queued.len());
                for slot in &mut chunk[..n] {
                    *slot = self.queued.pop_front().unwrap();
                }
                n
            };
            data.extend_from_slice(&chunk[..n]);
            if matched == 0 && !chunk[..n].contains(&END[0]) {
                continue;
            }
            for (i, &b) in chunk[..n].iter().enumerate() {
                matched = if b == END[matched] {
                    matched + 1
                } else {
                    usize::from(b == END[0])
                };
                if matched == END.len() {
                    let following = &chunk[i + 1..n];
                    if let Some(&key) = following.first() {
                        self.queued.push_front(key);
                    }
                    data.truncate(data.len() - END.len() - following.len());
                    break 'paste;
                }
            }
        }
        if memchr::memchr(b'\r', &data).is_some() {
            let mut after_cr = false;
            data.retain_mut(|b| {
                let keep = !(after_cr && *b == b'\n');
                after_cr = *b == b'\r';
                if after_cr {
                    *b = b'\n';
                }
                keep
            });
        }
        Ok(String::from_utf8(data)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
    }
}
fn multibyte_width(first: u8) -> Option<usize> {
    match first {
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

fn unbound_text(first: u8, session: &Session) -> bool {
    session.bindings.map != Keymap::Command
        && first >= 32
        && first != 127
        && !session.bindings.custom_prefix(&[first])
}

fn matching_opener(text: &str, cursor: usize) -> Option<usize> {
    let close = *text.as_bytes().get(cursor.checked_sub(1)?)?;
    let open = match close {
        b')' => b'(',
        b']' => b'[',
        b'}' => b'{',
        _ => return None,
    };
    let mut nested = 0;
    for (i, &byte) in text.as_bytes()[..cursor - 1].iter().enumerate().rev() {
        if byte == open {
            if nested == 0 {
                return Some(i);
            }
            nested -= 1;
        } else if byte == close {
            nested += 1;
        }
    }
    None
}

fn previous(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(i, _)| i)
}
fn rubout(text: &mut String, cursor: usize, overwrite: bool, undo: Option<&mut Undo>) -> usize {
    let start = previous(text, cursor);
    let blank = if overwrite && cursor < text.len() && start < cursor {
        let c = text[start..cursor].chars().next().unwrap();
        if c.is_control() {
            "  "
        } else {
            match c.width().unwrap_or(0) {
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
    i + s[i..].chars().next().map_or(0, char::len_utf8)
}

#[derive(Default, Clone, Copy)]
struct Position {
    row: usize,
    col: usize,
}

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
}

impl Screen {
    fn new(size: Option<(u16, u16)>) -> Self {
        let (rows, columns) = size.unwrap_or_else(|| {
            let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
            unsafe { libc::ioctl(2, libc::TIOCGWINSZ, &mut size) };
            (size.ws_row, size.ws_col)
        });
        Self {
            output: String::new(),
            row: 0,
            tail: None,
            height: if rows == 0 { 24 } else { usize::from(rows) },
            view: Default::default(),
            column: 0,
            had_hint: false,
            width: if columns < 4 {
                79
            } else {
                usize::from(columns) - 1
            },
        }
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
            let mut pos = Position::default();
            draw_prompt(prompt, self.width, &mut self.output, &mut pos);
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
        if text.len().saturating_add(suggestion.len()) >= 1024
            || text.len() >= self.height
                && memchr::memchr_iter(b'\n', text.as_bytes())
                    .nth(self.height.saturating_sub(3))
                    .is_some()
            || suggestion.len() >= self.height
                && memchr::memchr_iter(b'\n', suggestion.as_bytes())
                    .nth(self.height.saturating_sub(3))
                    .is_some()
        {
            return self.draw_window(prompt, text, cursor, suggestion);
        }
        self.view.invalidate();
        let out = &mut self.output;
        begin_redraw(out, self.row);
        let mut pos = Position::default();
        draw_prompt(prompt, self.width, out, &mut pos);
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
        io::stderr().write_all(out.as_bytes())
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
    let mut escape = 0;
    let mut hidden = false;
    for c in prompt.chars() {
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
        let n = text
            .bytes()
            .take_while(|b| (b' '..=b'~').contains(b))
            .count();
        if n == 0 {
            let c = text.chars().next().unwrap();
            draw_char(c, width, out, pos, false);
            text = &text[c.len_utf8()..];
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
        for ch in [
            '^',
            if c.is_ascii() {
                ((c as u8) ^ 64) as char
            } else {
                '?'
            },
        ] {
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
fn word_start(s: &str, mut i: usize) -> usize {
    for alnum in [false, true] {
        while let Some((j, c)) = s[..i].char_indices().next_back() {
            if c.is_alphanumeric() != alnum {
                break;
            }
            i = j;
        }
    }
    i
}
fn word_end(s: &str, mut i: usize) -> usize {
    for alnum in [false, true] {
        while let Some(c) = s[i..].chars().next() {
            if c.is_alphanumeric() != alnum {
                break;
            }
            i += c.len_utf8();
        }
    }
    i
}
pub enum Start<'a> {
    Command(String),
    Read { initial: &'a str, files_only: bool },
}

struct ReverseSearch {
    query: String,
    index: usize,
    original: String,
    original_cursor: usize,
}

fn reverse_search_prompt(query: &str, width: usize) -> String {
    let mut display = String::from("(reverse-i-search) `");
    let visible = if query.len() > 1024 {
        display.push('…');
        let mut start = query.len().saturating_sub(width * 2);
        while !query.is_char_boundary(start) {
            start += 1;
        }
        &query[start..]
    } else {
        query
    };
    for c in visible.chars() {
        if c.is_control() {
            display.extend(c.escape_default());
        } else {
            display.push(c);
        }
    }
    display.push_str("': ");
    display
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
    motion: bool,
    appended: Option<usize>,
    erased: Option<usize>,
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
        keys.insert_burst(first, session, &mut self.scratch)?;
        let inserted = &self.scratch;
        let append = (*cursor == text.len()).then_some(*cursor);
        let end = if self.overwrite && *cursor < text.len() {
            *cursor + char_offset(&text[*cursor..], inserted.chars().count())
        } else {
            *cursor
        };
        self.undo
            .record(text, *cursor..end, inserted.len(), *cursor, true);
        text.replace_range(*cursor..end, inserted);
        *cursor += inserted.len();
        Ok(append)
    }

    fn render_frame(
        &mut self,
        screen: &mut Screen,
        line: LineView<'_>,
        hint: &str,
        searching: bool,
        styled_or_multiline: bool,
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
            && text.len() < 1024
            && !text.contains('\n')
        {
            screen.move_short(prompt, text, cursor)?;
        } else if let Some(start) = self.appended.take()
            && (styled_or_multiline || start >= 1024)
            && let Some(pos) = screen.tail
            && screen.can_append(&text[start..])
            && cursor == text.len()
            && hint.is_empty()
            && !searching
        {
            screen.append(&text[start..], pos)?;
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
            write!(io::stderr(), "\x1b[{cells}D\x1b[K")?;
        } else {
            screen.draw(prompt, text, cursor, hint)?;
        }
        Ok(())
    }
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
    let mut term = Terminal::enter(session.bindings.paste())?;
    let mut keys = Keys {
        queued: std::mem::take(&mut session.editor_queue),
        ..Default::default()
    };
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
    let mut last_arg: Option<(usize, bool)> = None;
    let mut recall = recall::Recall::new(session.history_lines().len());
    let mut search: Option<ReverseSearch> = None;
    let mut pending_queries: Vec<SearchQuery> = Vec::new();
    let mut matched_query: Option<String> = None;
    let mut last_tab = false;
    let styled_or_multiline = prompt.contains(['\x1b', '\n']);
    let mut blink = false;
    let mut replaying = false;
    let mut view = View::Unread;
    if read.is_none() && keys.queued.is_empty() {
        history.prepare_search(session, &|| ready(0, 0));
    }
    loop {
        if session.bindings.map == Keymap::Command && !text.is_empty() {
            cursor = cursor.min(previous(&text, text.len()));
        }
        // Apply already visible typeahead to the completion before repainting.
        // A custom binding or control key must see the settled line first.
        replaying = replaying
            && keys.queued.front().is_some_and(|&b| {
                let width = match b {
                    8 | 32..=127 => 1,
                    _ => match multibyte_width(b) {
                        Some(width) => width,
                        None => return false,
                    },
                };
                keys.queued.len() >= width && !session.bindings.custom_prefix(&[b])
            });
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
        let search_prompt;
        let visible_prompt = if let Some(ReverseSearch { query, .. }) = &search {
            search_prompt = reverse_search_prompt(query, screen.width);
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
            styled_or_multiline,
            replaying,
        )?;
        if std::mem::take(&mut blink)
            && keys.queued.is_empty()
            && !ready(0, 0)
            && let Some(open) = matching_opener(&text, cursor)
        {
            screen.draw(visible_prompt, &text, open, hint)?;
            ready(0, 500);
            screen.draw(visible_prompt, &text, cursor, hint)?;
        }
        if let Some(last) = pending_queries.last() {
            let cached = last.skip == 0
                && matched_query
                    .as_ref()
                    .is_some_and(|prior| last.text.starts_with(prior) && text.contains(&last.text));
            let input_ready = || keys.input_pending();
            // Any newer match for a longer query was also a match for its
            // prefix. If the newest hit still fits, it remains the newest.
            if cached {
                matched_query = pending_queries.pop().map(|q| q.text);
                pending_queries.clear();
            } else if let Ok(result) =
                history.find_command(session, &mut view, &pending_queries, &input_ready)
            {
                let last = pending_queries.pop().unwrap();
                matched_query = result
                    .as_ref()
                    .filter(|hit| last.skip == 0 && hit.contains(&last.text))
                    .map(|_| last.text);
                pending_queries.clear();
                if let Some(found) = result
                    && found != text
                {
                    text = found;
                    cursor = text.len();
                    screen.draw(visible_prompt, &text, cursor, "")?;
                }
            }
        }
        if show_hint {
            let input_ready = || keys.input_pending();
            // After showing a recalled command, let the terminal paint it or send
            // the next arrow before searching older rows for a longer suggestion.
            if needs_hint
                && !(recall.on_match(session.history_lines().len())
                    && (!keys.queued.is_empty() || ready(0, 1)))
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
            io::stderr().write_all(b"\r\n")?;
            return Ok(Input::Cancel);
        }
        if session.notify && keys.queued.is_empty() {
            crate::wait_for_job_notice(&mut session.jobs, |jobs| {
                screen.draw("", "", 0, "")?;
                jobs.report_changes(session.posix);
                screen.draw(visible_prompt, &text, cursor, "")
            })?;
        }
        let first = match keys.get() {
            Ok(Some(c)) => c,
            Ok(None) => return Ok(Input::End),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                io::stderr().write_all(b"\r\n")?;
                return Ok(Input::Cancel);
            }
            Err(e) => return Err(e),
        };
        // Ordinary committed text can bypass key-sequence and editing-action dispatch.
        if search.is_none() && unbound_text(first, session) {
            last_arg = None;
            recall.edited(session.history_lines().len());
            edit.appended = edit.insert(&mut keys, first, session, &mut text, &mut cursor)?;
            blink = session.bindings.blink_matching_paren
                && matches!(edit.scratch.as_bytes(), b")" | b"]" | b"}");
            last_tab = false;
            continue;
        }
        let (binding, right_arrow) = keys.binding(first, session)?;
        let action = match binding {
            Binding::Removed => {
                vi = vi::State::default();
                continue;
            }
            Binding::Macro(seq) => {
                keys.expansions += 1;
                if keys.expansions > 64 || keys.queued.len().saturating_add(seq.len()) > 65536 {
                    keys.queued.clear();
                    io::stderr().write_all(b"\x07")?;
                } else {
                    for key in seq.into_iter().rev() {
                        keys.queued.push_front(key);
                    }
                }
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
                last_arg = None;
                recall = recall::Recall::new(session.history_lines().len());
                if let crate::Flow::Exit(code) = flow {
                    return Ok(Input::Exit(code));
                }
                term = Terminal::enter(session.bindings.paste())?;
                screen.row = 0;
                screen.view.invalidate();
                continue;
            }
            Binding::Edit(a) => a,
        };
        if action != Action::LastArg {
            last_arg = None;
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
            matched_query = None;
            search = Some(ReverseSearch {
                query: String::new(),
                index: 0,
                original: text.clone(),
                original_cursor: cursor,
            });
            // Readline keeps the current draft until a nonempty query matches.
            continue;
        }
        if let Some(ReverseSearch {
            query,
            index,
            original,
            original_cursor,
        }) = search.as_mut()
        {
            match action {
                Action::Abort => {
                    pending_queries.clear();
                    text = std::mem::take(original);
                    cursor = *original_cursor;
                    search = None;
                    continue;
                }
                Action::Accept | Action::Forward | Action::Back | Action::Home | Action::End => {
                    if !pending_queries.is_empty()
                        && let Ok(Some(found)) =
                            history.find_command(session, &mut view, &pending_queries, &|| false)
                    {
                        text = found;
                    }
                    pending_queries.clear();
                    search = None;
                    cursor = text.len();
                }
                _ => {
                    let mut shortest = None;
                    if action == Action::Search {
                        *index += 1;
                    } else if action == Action::Rubout {
                        query.pop();
                        *index = 0;
                    } else if action == Action::Insert {
                        keys.insert_burst(first, session, &mut edit.scratch)?;
                        if let Some(c) = edit.scratch.chars().next() {
                            shortest = Some(query.len() + c.len_utf8());
                            query.push_str(&edit.scratch);
                            *index = 0;
                        }
                    } else if action == Action::Paste {
                        query.push_str(
                            &keys
                                .paste(|bytes| screen.paste_frame(visible_prompt, "", 0, bytes))?,
                        );
                        *index = 0;
                    }
                    if matches!(
                        action,
                        Action::Complete
                            | Action::Up
                            | Action::Down
                            | Action::PrefixUp
                            | Action::PrefixDown
                    ) || (!query.chars().any(char::is_control)
                        && crate::history::fragments(query).nth(1).is_some())
                    {
                        let selected = picker::history(
                            std::mem::take(query),
                            history,
                            session,
                            &mut keys,
                            &mut screen,
                        );
                        pending_queries.clear();
                        match selected {
                            Ok(Some(command)) => {
                                edit.undo = Undo::default();
                                text = command;
                                cursor = text.len();
                            }
                            Ok(None) => {
                                text = std::mem::take(original);
                                cursor = *original_cursor;
                            }
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                                screen.draw(prompt, original, *original_cursor, "")?;
                                io::stderr().write_all(b"\r\n")?;
                                return Ok(Input::Cancel);
                            }
                            Err(e) => return Err(e),
                        }
                        search = None;
                        last_tab = false;
                        continue;
                    }
                    pending_queries.push(SearchQuery {
                        text: query.clone(),
                        skip: *index,
                        shortest: shortest.unwrap_or(query.len()),
                    });
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
                let vi = action == Action::ViEditAndExecute;
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
                drop(term);
                io::stderr().write_all(b"\r\n")?;
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                match crate::history::commands::edit_text(session, editor, &text, status) {
                    Ok(Ok(mut edited)) => {
                        if !edited.ends_with('\n') {
                            edited.push('\n');
                        }
                        session.editor_queue = std::mem::take(&mut keys.queued);
                        return Ok(Input::Line(edited));
                    }
                    Ok(Err(crate::Flow::Exit(code))) => return Ok(Input::Exit(code)),
                    Ok(Err(_)) => return Ok(Input::Cancel),
                    Err(e) => {
                        diagnostic!("{editor}: {e}");
                        return Ok(Input::Cancel);
                    }
                }
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
                return Ok(Input::End);
            }
            Action::Accept | Action::Comment => {
                if action == Action::Comment {
                    text.insert_str(0, session.bindings.comment_begin.as_deref().unwrap_or("#"));
                }
                if action == Action::Comment
                    || visible_prompt != prompt
                    || (!styled_or_multiline && text.len() < 1024)
                    || replaying
                    || screen.tail.is_none()
                {
                    screen.draw(prompt, &text, text.len(), "")?;
                }
                io::stderr().write_all(b"\r\n")?;
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
                    && start < cursor
                    && (styled_or_multiline || start >= 1024)
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
            Action::LastArg => {
                let (end, replace) = last_arg.unwrap_or((history_position, false));
                if replace {
                    edit.undo.apply(&mut text, &mut cursor);
                }
                let next = end.checked_sub(1);
                let arg = next
                    .and_then(|i| session.history_lines().get(i))
                    .and_then(|line| crate::parser::last_history_argument(line));
                if let Some(arg) = arg {
                    mark = if text[..cursor].is_ascii() {
                        cursor
                    } else {
                        text[..cursor].chars().count()
                    };
                    edit.undo
                        .record(&text, cursor..cursor, arg.len(), cursor, false);
                    text.insert_str(cursor, arg);
                    cursor += arg.len();
                }
                last_arg = Some((next.unwrap_or(0), arg.is_some()));
            }
            Action::Yank => {
                edit.undo.record(
                    &text,
                    cursor..cursor,
                    session.editor_kill.len(),
                    cursor,
                    false,
                );
                text.insert_str(cursor, &session.editor_kill);
                cursor += session.editor_kill.len();
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
                    edit.undo
                        .record(&text, cursor..cursor, end - cursor, cursor, false);
                    text.push_str(&line[cursor..end]);
                    cursor = text.len();
                }
            }
            Action::ForwardWord => cursor = word_end(&text, cursor),
            Action::Reload => bindings::reload(session),
            Action::TabInsert => {
                edit.undo.record(&text, cursor..cursor, 1, cursor, false);
                text.insert(cursor, '\t');
                cursor += 1;
            }
            Action::HistoryExpand | Action::MagicSpace => {
                if let Ok(result) = crate::history::expand::expand(session, &text) {
                    let at_start = cursor == 0;
                    let suffix = text.len() - cursor;
                    if let std::borrow::Cow::Owned(expanded) = result.text {
                        let old_end = text.len();
                        edit.undo.replace(&mut text, 0..old_end, &expanded, cursor);
                        cursor = text.len();
                    }
                    if action == Action::MagicSpace {
                        cursor = if at_start {
                            0
                        } else {
                            text.len().saturating_sub(suffix)
                        };
                        while !text.is_char_boundary(cursor) {
                            cursor -= 1;
                        }
                        edit.undo.record(&text, cursor..cursor, 1, cursor, false);
                        text.insert(cursor, ' ');
                        cursor += 1;
                    }
                }
            }
            Action::UpperWord | Action::LowerWord | Action::CapitalWord => {
                cursor =
                    edit.change_word_case(action, &mut text, cursor, session.character_locale());
            }
            Action::TransposeWords | Action::Transpose => {
                cursor = edit.transpose(action, &mut text, cursor);
            }
            Action::Quote => {
                if let Some(key) = keys.get()?
                    && let Some(c) = keys.character(key)?
                {
                    edit.undo
                        .record(&text, cursor..cursor, c.len_utf8(), cursor, false);
                    text.insert(cursor, c);
                    cursor += c.len_utf8();
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
                        let selected =
                            picker::choose(&result.choices, &mut keys, &mut screen, session);
                        replaying = false;
                        match selected {
                            Ok(Some(index)) => {
                                let word = result.select(index);
                                edit.undo
                                    .replace(&mut text, result.start..cursor, &word, cursor);
                                cursor = result.start + word.len();
                            }
                            Ok(None) => {}
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                                screen.draw(prompt, &text, cursor, "")?;
                                io::stderr().write_all(b"\r\n")?;
                                return Ok(Input::Cancel);
                            }
                            Err(e) => return Err(e),
                        }
                    } else if was_tab && result.choices.len() > 1 {
                        screen.draw(prompt, &text, text.len(), "")?;
                        io::stderr().write_all(b"\r\n")?;
                        let mut display = String::new();
                        for (i, choice) in result.choices.iter().enumerate() {
                            if i > 0 {
                                display.push_str("  ");
                            }
                            for c in choice.chars() {
                                if c.is_control() {
                                    display.extend(c.escape_default());
                                } else {
                                    display.push(c);
                                }
                            }
                        }
                        io::stderr().write_all(display.as_bytes())?;
                        io::stderr().write_all(b"\r\n")?;
                        screen.row = 0;
                        screen.view.invalidate();
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
                recall.navigate(action, session.history_lines(), &mut text, &mut cursor);
                if recall.position() != history_position {
                    edit.undo = Undo::default();
                }
            }
            Action::PreviousDirectory | Action::NextDirectory => unreachable!(),
            Action::Forward => {
                if cursor == text.len() {
                    if !text.is_empty() {
                        history.ensure(session, &mut view, false, &|| false);
                        if let Some(s) =
                            history.suggestion(session.history_lines(), &text, true, || true)
                        {
                            edit.undo
                                .record(&text, cursor..cursor, s.len(), cursor, false);
                            text.push_str(s);
                        } else if let Some(s) =
                            history.older_suggestion(session, &mut view, &text, &|| false)
                        {
                            edit.undo
                                .record(&text, cursor..cursor, s.len(), cursor, false);
                            text.push_str(s);
                        }
                        cursor = text.len();
                    }
                } else {
                    cursor = next(&text, cursor);
                }
            }
            Action::Paste => {
                if cursor == text.len() {
                    edit.appended = Some(cursor);
                }
                let pasted =
                    keys.paste(|bytes| screen.paste_frame(prompt, &text, cursor, bytes))?;
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
                edit.appended = edit.insert(&mut keys, first, session, &mut text, &mut cursor)?;
            }
            _ => {}
        }
        if edit.motion && matches!(action, Action::Back | Action::Forward) && search.is_none() {
            cursor = keys.motion_burst(&text, cursor, session);
        }
    }
}
