use std::io::{self, Write};
use std::os::fd::RawFd;
use unicode_width::UnicodeWidthChar;

use crate::{Session, complete, history::History};

pub enum Input {
    Line(String),
    End,
    Cancel,
}

struct Terminal(libc::termios);

impl Terminal {
    fn enter() -> io::Result<Self> {
        let mut old = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(0, &mut old) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut new = old;
        new.c_lflag &= !(libc::ICANON | libc::ECHO | libc::IEXTEN);
        new.c_iflag &= !(libc::IXON | libc::ICRNL);
        new.c_cc[libc::VMIN] = 1;
        new.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &new) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(old))
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = io::stderr().write_all(b"\x1b[?2004l");
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.0);
        }
    }
}

fn byte() -> io::Result<Option<u8>> {
    let mut key = 0;
    let n = unsafe { libc::read(0, (&mut key as *mut u8).cast(), 1) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((n == 1).then_some(key))
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

fn sequence() -> io::Result<Vec<u8>> {
    let mut result = Vec::new();
    while result.len() < 12 && ready(0, 35) {
        let Some(c) = byte()? else {
            break;
        };
        result.push(c);
        if result.len() == 1 && !matches!(c, b'[' | b'O') {
            break;
        }
        if result.len() > 1 && (0x40..=0x7e).contains(&c) {
            break;
        }
    }
    Ok(result)
}

fn paste() -> io::Result<String> {
    let mut data = Vec::new();
    const END: &[u8] = b"\x1b[201~";
    while let Some(c) = byte()? {
        data.push(c);
        if data.ends_with(END) {
            data.truncate(data.len() - END.len());
            break;
        }
    }
    Ok(String::from_utf8_lossy(&data)
        .replace("\r\n", "\n")
        .replace('\r', "\n"))
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
        let mut out = String::from("\r");
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
                draw_char(c, self.width, &mut out, &mut pos, true);
            }
        }
        for c in text[..cursor].chars() {
            draw_char(c, self.width, &mut out, &mut pos, false);
        }
        let caret = pos;
        for c in text[cursor..].chars() {
            draw_char(c, self.width, &mut out, &mut pos, false);
        }
        if !suggestion.is_empty() {
            out.push_str("\x1b[2m");
            for c in suggestion.chars() {
                draw_char(c, self.width, &mut out, &mut pos, false);
            }
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

pub fn line(prompt: &str, history: &mut History, session: &mut Session) -> io::Result<Input> {
    let _term = Terminal::enter()?;
    io::stderr().write_all(b"\x1b[?2004h")?;
    let mut screen = Screen::new();
    let mut text = String::new();
    let mut cursor = 0;
    let mut hist = history.local.len();
    let mut draft = String::new();
    let mut search: Option<(String, usize, String)> = None;
    let mut last_tab = false;
    let mut loaded_history = false;
    loop {
        let hint = if search.is_none() && cursor == text.len() {
            history.suggestion(&text).unwrap_or_default()
        } else {
            ""
        };
        let search_prompt;
        let visible_prompt = if let Some((query, _, _)) = &search {
            search_prompt = format!("(reverse-i-search) `{query}': ");
            &search_prompt
        } else {
            prompt
        };
        screen.draw(visible_prompt, &text, cursor, hint)?;
        let key = match byte() {
            Ok(Some(c)) => c,
            Ok(None) => return Ok(Input::End),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                io::stderr().write_all(b"\r\n")?;
                return Ok(Input::Cancel);
            }
            Err(e) => return Err(e),
        };
        if !loaded_history && key >= 32 {
            history.refresh(session);
            loaded_history = true;
        }
        let entering_search = key == 18 && search.is_none();
        if entering_search {
            history.refresh(session);
            search = Some((String::new(), 0, text.clone()));
        }
        if let Some((query, index, original)) = search.as_mut() {
            match key {
                7 | 27 => {
                    text = std::mem::take(original);
                    search = None;
                    cursor = text.len();
                    continue;
                }
                13 | 10 => {
                    search = None;
                    cursor = text.len();
                }
                _ => {
                    if key == 18 {
                        if !entering_search {
                            *index += 1;
                        }
                    } else if key == 127 || key == 8 {
                        query.pop();
                        *index = 0;
                    } else if key >= 32 {
                        if let Some(c) = character(key)? {
                            query.push(c);
                            *index = 0;
                        }
                    }
                    if let Some(found) = history.search(query, *index) {
                        text = found.to_string();
                    }
                    cursor = text.len();
                    continue;
                }
            }
        }
        let was_tab = last_tab;
        last_tab = key == 9;
        match key {
            3 => {
                io::stderr().write_all(b"\r\n")?;
                return Ok(Input::Cancel);
            }
            4 if text.is_empty() => {
                io::stderr().write_all(b"\r\n")?;
                return Ok(Input::End);
            }
            13 | 10 => {
                screen.draw(prompt, &text, text.len(), "")?;
                io::stderr().write_all(b"\r\n")?;
                text.push('\n');
                return Ok(Input::Line(text));
            }
            1 => cursor = 0,
            5 => cursor = text.len(),
            2 => cursor = previous(&text, cursor),
            6 => cursor = next(&text, cursor),
            4 => {
                text.replace_range(cursor..next(&text, cursor), "");
            }
            8 | 127 => {
                let start = previous(&text, cursor);
                text.replace_range(start..cursor, "");
                cursor = start;
            }
            11 => text.truncate(cursor),
            21 => {
                text.replace_range(..cursor, "");
                cursor = 0;
            }
            23 => {
                let start = text[..cursor]
                    .trim_end()
                    .rfind(char::is_whitespace)
                    .map_or(0, |i| next(&text, i));
                text.replace_range(start..cursor, "");
                cursor = start;
            }
            12 => {
                io::stderr().write_all(b"\x1b[2J\x1b[H")?;
                screen.row = 0;
            }
            9 => {
                if let Some(result) = complete::at(&text, cursor, session) {
                    if was_tab && result.choices.len() > 1 {
                        screen.draw(prompt, &text, text.len(), "")?;
                        io::stderr().write_all(b"\r\n")?;
                        let display = result
                            .choices
                            .iter()
                            .map(|s| {
                                s.chars()
                                    .map(|c| {
                                        if c.is_control() {
                                            c.escape_default().to_string()
                                        } else {
                                            c.to_string()
                                        }
                                    })
                                    .collect::<String>()
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        io::stderr().write_all(display.as_bytes())?;
                        io::stderr().write_all(b"\r\n")?;
                        screen.row = 0;
                    } else {
                        text.replace_range(result.start..cursor, &result.replacement);
                        cursor = result.start + result.replacement.len();
                    }
                }
            }
            27 => {
                let seq = sequence()?;
                match seq.as_slice() {
                    b"[A" | b"OA" => {
                        if hist == history.local.len() {
                            draft = text.clone();
                        }
                        if hist > 0 {
                            hist -= 1;
                            text = history.local[hist].clone();
                            cursor = text.len();
                        }
                    }
                    b"[B" | b"OB" => {
                        if hist < history.local.len() {
                            hist += 1;
                            text = history.local.get(hist).unwrap_or(&draft).clone();
                            cursor = text.len();
                        }
                    }
                    b"[D" | b"OD" => cursor = previous(&text, cursor),
                    b"[C" | b"OC" => {
                        if cursor == text.len() {
                            if let Some(s) = history.suggestion(&text) {
                                text.push_str(s);
                                cursor = text.len();
                            }
                        } else {
                            cursor = next(&text, cursor);
                        }
                    }
                    b"[H" | b"OH" | b"[1~" => cursor = 0,
                    b"[F" | b"OF" | b"[4~" => cursor = text.len(),
                    b"[3~" => {
                        text.replace_range(cursor..next(&text, cursor), "");
                    }
                    b"[200~" => {
                        let inserted = paste()?;
                        text.insert_str(cursor, &inserted);
                        cursor += inserted.len();
                    }
                    _ => {}
                }
            }
            c if c >= 32 => {
                if let Some(c) = character(c)? {
                    text.insert(cursor, c);
                    cursor += c.len_utf8();
                }
            }
            _ => {}
        }
    }
}

fn character(first: u8) -> io::Result<Option<char>> {
    let width = match first {
        0..=127 => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Ok(None),
    };
    let mut bytes = [first; 4];
    for item in &mut bytes[1..width] {
        let Some(c) = byte()? else {
            return Ok(None);
        };
        *item = c;
    }
    Ok(std::str::from_utf8(&bytes[..width])
        .ok()
        .and_then(|s| s.chars().next()))
}
