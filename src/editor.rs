pub(crate) mod bindings;
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

fn ready(fd: RawFd, ms: i32) -> bool {
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

    fn character(&mut self, first: u8) -> io::Result<Option<char>> {
        let width = match first {
            0..=127 => return Ok(Some(char::from(first))),
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Ok(None),
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
        if session.bindings.map != Keymap::Command
            && first >= 32
            && first != 127
            && !session.bindings.custom_prefix(&[first])
        {
            return Ok((Binding::Edit(Action::Insert), false));
        }
        let mut seq = vec![first];
        let mut best = None;
        loop {
            let (exact, longer) = session.bindings.lookup(&seq);
            if let Some(action) = exact {
                best = Some((seq.len(), action));
            }
            if !longer || self.queued.is_empty() && !ready(0, session.bindings.timeout()) {
                break;
            }
            let Some(key) = self.get()? else { break };
            seq.push(key);
        }
        let (len, action) = best.unwrap_or((
            1,
            if session.bindings.map != Keymap::Command && first >= 32 && first != 127 {
                Binding::Edit(Action::Insert)
            } else {
                Binding::Removed
            },
        ));
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
    fn paste(&mut self) -> io::Result<String> {
        const END: &[u8; 6] = b"\x1b[201~";
        let mut data = Vec::new();
        let mut matched = 0;
        let mut chunk = [0; END.len() + 1];
        'paste: loop {
            if crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
                || crate::traps::input_interrupted()
            {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let limit = chunk.len() - matched;
            let n = if self.queued.is_empty() {
                let immediate = unsafe { libc::read(0, chunk.as_mut_ptr().cast(), limit) };
                if immediate > 0 {
                    immediate as usize
                } else {
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
fn previous(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(i, _)| i)
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
}

impl Screen {
    fn new() -> Self {
        let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
        unsafe {
            libc::ioctl(2, libc::TIOCGWINSZ, &mut size);
        }
        Self {
            output: String::new(),
            row: 0,
            width: if size.ws_col < 4 {
                79
            } else {
                usize::from(size.ws_col) - 1
            },
        }
    }

    fn draw(
        &mut self,
        prompt: &str,
        text: &str,
        cursor: usize,
        suggestion: &str,
    ) -> io::Result<()> {
        let out = &mut self.output;
        out.clear();
        out.push('\r');
        if self.row > 0 {
            out.push_str(&format!("\x1b[{}A", self.row));
        }
        out.push_str("\x1b[J");
        let mut pos = Position::default();
        let mut escape = 0;
        for c in prompt.chars() {
            if escape != 0 || c == '\x1b' {
                out.push(c);
                escape = match (escape, c) {
                    (0, _) => 1,
                    (1, '[') => 2,
                    (1, ']') => 3,
                    (2, '@'..='~') | (3, '\x07') | (4, '\\') => 0,
                    (3, '\x1b') => 4,
                    (2, _) => 2,
                    (3 | 4, _) => 3,
                    _ => 0,
                };
            } else {
                draw_char(c, self.width, out, &mut pos, true);
            }
        }
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
                out.push_str(&format!("\x1b[{}A", pos.row - caret.row));
            }
            out.push('\r');
            if caret.col > 0 {
                out.push_str(&format!("\x1b[{}C", caret.col));
            }
        }
        self.row = caret.row;
        io::stderr().write_all(out.as_bytes())
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
pub fn line(
    prompt: &str,
    history: &mut History,
    session: &mut Session,
    status: u8,
    read: Option<(&str, bool)>,
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
    let mut mark = 0usize;
    let mut screen = Screen::new();
    let mut text = read.map_or("", |r| r.0).to_owned();
    let mut cursor = text.len();
    let mut hist = session.history_lines().len();
    let mut draft = String::new();
    let mut search: Option<(String, usize, String)> = None;
    let mut pending_queries = Vec::new();
    let mut inserted = String::new();
    let mut last_tab = false;
    let mut view = View::Unread;
    loop {
        if session.bindings.map == Keymap::Command && !text.is_empty() {
            cursor = cursor.min(previous(&text, text.len()));
        }
        let show_hint =
            read.is_none() && search.is_none() && cursor == text.len() && !text.is_empty();
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
        let visible_prompt = if let Some((query, _, _)) = &search {
            let mut display = String::from("(reverse-i-search) `");
            for c in query.chars() {
                if c.is_control() {
                    display.extend(c.escape_default());
                } else {
                    display.push(c);
                }
            }
            display.push_str("': ");
            search_prompt = display;
            &search_prompt
        } else {
            prompt
        };
        let needs_hint = hint.is_empty();
        screen.draw(visible_prompt, &text, cursor, hint)?;
        if !pending_queries.is_empty() {
            let input_ready = || {
                !keys.queued.is_empty()
                    || ready(0, 0)
                    || crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
            };
            if let Ok(result) =
                history.find_command(session, &mut view, &pending_queries, &input_ready)
            {
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
            let input_ready = || {
                !keys.queued.is_empty()
                    || ready(0, 0)
                    || crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
            };
            if needs_hint && history.ensure(session, &mut view, false, &input_ready) {
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
        let first = match keys.get() {
            Ok(Some(c)) => c,
            Ok(None) => return Ok(Input::End),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                io::stderr().write_all(b"\r\n")?;
                return Ok(Input::Cancel);
            }
            Err(e) => return Err(e),
        };
        let (binding, right_arrow) = keys.binding(first, session)?;
        let action = match binding {
            Binding::Removed => continue,
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
                let vars = session.bind(&[
                    ("READLINE_LINE".into(), text.clone()),
                    (
                        "READLINE_POINT".into(),
                        text[..cursor].chars().count().to_string(),
                    ),
                    ("READLINE_MARK".into(), mark.to_string()),
                ]);
                let flow = crate::eval_text(session, &command, status);
                text = session.get("READLINE_LINE").unwrap_or_default();
                cursor = char_offset(
                    &text,
                    session
                        .get("READLINE_POINT")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                );
                mark = session
                    .get("READLINE_MARK")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                session.restore(vars);
                if let crate::Flow::Exit(code) = flow {
                    return Ok(Input::Exit(code));
                }
                term = Terminal::enter(session.bindings.paste())?;
                screen.row = 0;
                continue;
            }
            Binding::Edit(a) => a,
        };
        let entering_search = action == Action::Search && search.is_none();
        if entering_search {
            search = Some((String::new(), 0, text.clone()));
        }
        if let Some((query, index, original)) = search.as_mut() {
            match action {
                Action::Abort => {
                    pending_queries.clear();
                    text = std::mem::take(original);
                    search = None;
                    cursor = text.len();
                    continue;
                }
                Action::Accept => {
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
                        if !entering_search {
                            *index += 1;
                        }
                    } else if action == Action::Rubout {
                        query.pop();
                        *index = 0;
                    } else if action == Action::Insert {
                        keys.insert_burst(first, session, &mut inserted)?;
                        if let Some(c) = inserted.chars().next() {
                            shortest = Some(query.len() + c.len_utf8());
                            query.push_str(&inserted);
                            *index = 0;
                        }
                    } else if action == Action::Paste {
                        query.push_str(&keys.paste()?);
                        *index = 0;
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
        let was_tab = last_tab;
        last_tab = action == Action::Complete;
        match action {
            Action::Redraw => {}
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
                    text.insert(0, '#');
                }
                screen.draw(prompt, &text, text.len(), "")?;
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
                text.replace_range(cursor..next(&text, cursor), "");
            }
            Action::Rubout => {
                let start = previous(&text, cursor);
                text.replace_range(start..cursor, "");
                cursor = start;
            }
            Action::Kill
            | Action::Discard
            | Action::KillBack
            | Action::RubWord
            | Action::KillBackWord
            | Action::KillWord
            | Action::Cut
            | Action::Copy => {
                let point = char_offset(&text, mark);
                let (lo, hi) = match action {
                    Action::Kill => (cursor, text.len()),
                    Action::KillWord => (cursor, word_end(&text, cursor)),
                    Action::RubWord => (
                        text[..cursor]
                            .trim_end()
                            .rfind(char::is_whitespace)
                            .map_or(0, |i| next(&text, i)),
                        cursor,
                    ),
                    Action::KillBackWord => (word_start(&text, cursor), cursor),
                    Action::Cut | Action::Copy => (point.min(cursor), point.max(cursor)),
                    _ => (0, cursor),
                };
                if lo != hi {
                    session.editor_kill.clear();
                    session.editor_kill.push_str(&text[lo..hi]);
                    if action != Action::Copy {
                        text.replace_range(lo..hi, "");
                        cursor = lo;
                    }
                }
            }
            Action::Yank => {
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
            Action::ForwardWord => cursor = word_end(&text, cursor),
            Action::Transpose if cursor > 0 => {
                let end = if cursor == text.len() {
                    cursor
                } else {
                    next(&text, cursor)
                };
                let mid = previous(&text, end);
                let start = previous(&text, mid);
                let replacement = format!("{}{}", &text[mid..end], &text[start..mid]);
                text.replace_range(start..end, &replacement);
                cursor = end;
            }
            Action::Quote => {
                if let Some(key) = keys.get()?
                    && let Some(c) = keys.character(key)?
                {
                    text.insert(cursor, c);
                    cursor += c.len_utf8();
                }
            }
            Action::Clear => {
                io::stderr().write_all(b"\x1b[2J\x1b[H")?;
                screen.row = 0;
            }
            Action::Complete | Action::List if !session.bindings.disable_completion => {
                let was_tab = was_tab || action == Action::List;
                if let Some(result) =
                    complete::at(&text, cursor, session, was_tab, read.is_some_and(|r| r.1))
                {
                    if was_tab && result.choices.len() > 1 {
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
                    } else {
                        text.replace_range(result.start..cursor, &result.replacement);
                        cursor = result.start + result.replacement.len();
                    }
                }
            }
            Action::Up | Action::FirstHistory => {
                if hist == session.history_lines().len() {
                    draft = text.clone();
                }
                if hist > 0 {
                    hist = if action == Action::FirstHistory {
                        0
                    } else {
                        hist - 1
                    };
                    text = session.history_lines()[hist].clone();
                    cursor = text.len();
                }
            }
            Action::Down | Action::LastHistory => {
                if hist < session.history_lines().len() {
                    hist = if action == Action::LastHistory {
                        session.history_lines().len()
                    } else {
                        hist + 1
                    };
                    text = session.history_lines().get(hist).unwrap_or(&draft).clone();
                    cursor = text.len();
                }
            }
            Action::Forward => {
                if cursor == text.len() {
                    if !text.is_empty() {
                        history.ensure(session, &mut view, false, &|| false);
                        if let Some(s) =
                            history.suggestion(session.history_lines(), &text, true, || true)
                        {
                            text.push_str(s);
                        } else if let Some(s) =
                            history.older_suggestion(session, &mut view, &text, &|| false)
                        {
                            text.push_str(s);
                        }
                        cursor = text.len();
                    }
                } else {
                    cursor = next(&text, cursor);
                }
            }
            Action::Paste => {
                let pasted = keys.paste()?;
                let end = cursor + pasted.len();
                if text.is_empty() {
                    text = pasted;
                } else {
                    text.insert_str(cursor, &pasted);
                }
                cursor = end;
            }
            Action::Insert => {
                keys.insert_burst(first, session, &mut inserted)?;
                text.insert_str(cursor, &inserted);
                cursor += inserted.len();
            }
            _ => {}
        }
    }
}
