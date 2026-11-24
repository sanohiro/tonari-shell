// The picker owns only its query. The draft and its cursor stay in the editor
// until the user accepts a single, normally quoted completion.
use super::{Keys, Screen, ready};
use crate::Session;
use std::fmt::Write as _;
use std::io::{self, Write as _};
use unicode_width::UnicodeWidthChar;

fn label(out: &mut String, text: &str, mut cells: usize) -> usize {
    let start = cells;
    for c in text.chars() {
        if c.is_control() {
            let escaped = c.escape_default();
            if escaped.len() > cells {
                break;
            }
            cells -= escaped.len();
            out.extend(escaped);
        } else {
            let w = c.width().unwrap_or(0);
            if w > cells {
                break;
            }
            out.push(c);
            cells -= w;
        }
    }
    start - cells
}

fn draw(
    screen: &mut Screen,
    choices: &[String],
    hits: &[usize],
    selected: usize,
    query: &str,
    rows: usize,
) {
    let offset = selected / rows * rows;
    let out = &mut screen.output;
    out.clear();
    out.push('\r');
    if screen.row > 0 {
        let _ = write!(out, "\x1b[{}A", screen.row);
    }
    out.push_str("\x1b[J");
    let prefix = label(out, "filter> ", screen.width);
    let available = screen.width - prefix;
    let mut remaining = available;
    let mut start = query.len();
    for (i, c) in query.char_indices().rev() {
        let w = if c.is_control() {
            c.escape_default().len()
        } else {
            c.width().unwrap_or(0)
        };
        if w > remaining {
            break;
        }
        remaining -= w;
        start = i;
    }
    let caret = prefix + label(out, &query[start..], available);
    let mut shown = 0;
    for (row, &index) in hits.iter().skip(offset).take(rows).enumerate() {
        out.push_str("\r\n");
        let prefix = label(
            out,
            if offset + row == selected { "> " } else { "  " },
            screen.width,
        );
        label(out, &choices[index], screen.width - prefix);
        shown += 1;
    }
    if hits.is_empty() {
        out.push_str("\r\n");
        label(out, "(no matches)", screen.width);
        shown = 1;
    }
    let _ = write!(out, "\x1b[{shown}A\r");
    if caret != 0 {
        let _ = write!(out, "\x1b[{caret}C");
    }
    screen.row = 0;
    screen.tail = None;
    screen.view.invalidate();
}

enum Key {
    Text,
    Paste,
    Up,
    Down,
    Accept,
    Erase,
    Cancel,
    Ignore,
}

fn read_key(first: u8, keys: &mut Keys, session: &Session) -> io::Result<Key> {
    Ok(match first {
        b'\r' | b'\n' => Key::Accept,
        b'\t' | 14 => Key::Down,
        16 => Key::Up,
        7 => Key::Cancel,
        8 | 127 => Key::Erase,
        27 => {
            // Disambiguate a lone Escape from an arrow or bracketed paste.
            let timeout = session.bindings.timeout();
            if keys.queued.is_empty() && !ready(0, timeout.min(50)) {
                return Ok(Key::Cancel);
            }
            let Some(introducer) = keys.get()? else {
                return Ok(Key::Cancel);
            };
            if !matches!(introducer, b'[' | b'O') {
                keys.queued.push_front(introducer);
                return Ok(Key::Cancel);
            }
            let mut params = [0u8; 3];
            let mut len = 0;
            loop {
                if keys.queued.is_empty() && !ready(0, timeout) {
                    return Ok(Key::Ignore);
                }
                let Some(b) = keys.get()? else {
                    return Ok(Key::Cancel);
                };
                match b {
                    0x20..=0x3f => {
                        if let Some(slot) = params.get_mut(len) {
                            *slot = b;
                        }
                        len += 1;
                    }
                    0x40..=0x7e => {
                        return Ok(match (b, len) {
                            (b'A' | b'Z', 0) => Key::Up,
                            (b'B', 0) => Key::Down,
                            (b'~', 3) if introducer == b'[' && &params == b"200" => Key::Paste,
                            _ => Key::Ignore,
                        });
                    }
                    _ => {
                        keys.queued.push_front(b);
                        return Ok(Key::Ignore);
                    }
                }
            }
        }
        32.. => Key::Text,
        _ => {
            keys.queued.push_front(first);
            Key::Cancel
        }
    })
}

pub(super) fn choose(
    choices: &[String],
    keys: &mut Keys,
    screen: &mut Screen,
    session: &Session,
) -> io::Result<Option<usize>> {
    let mut query = String::new();
    let mut inserted = String::new();
    let mut hits = Vec::new();
    let mut scanned = 0;
    let mut selected: usize = 0;
    let rows = screen.height.saturating_sub(1).clamp(1, 8);
    loop {
        // Preserve source order, and resume scanning only when the user
        // navigates past the matches already found for this query.
        let wanted = (selected / rows + 1) * rows;
        if hits.len() < wanted && scanned < choices.len() {
            if query.is_empty() {
                let end = wanted.min(choices.len());
                hits.extend(scanned..end);
                scanned = end;
            } else {
                let finder = memchr::memmem::Finder::new(query.as_bytes());
                while scanned < choices.len() && hits.len() < wanted {
                    let index = scanned;
                    scanned += 1;
                    if finder.find(choices[index].as_bytes()).is_some() {
                        hits.push(index);
                    }
                }
            }
        }
        selected = selected.min(hits.len().saturating_sub(1));
        draw(screen, choices, &hits, selected, &query, rows);
        io::stderr().write_all(screen.output.as_bytes())?;
        let Some(first) = keys.get()? else {
            return Ok(None);
        };
        match read_key(first, keys, session)? {
            Key::Text => {
                keys.insert_burst(first, session, &mut inserted)?;
                query.push_str(&inserted);
            }
            Key::Paste => query.push_str(&keys.paste(|_| Ok(()))?),
            Key::Erase => {
                query.pop();
            }
            Key::Cancel => return Ok(None),
            Key::Accept => {
                if let Some(&index) = hits.get(selected) {
                    return Ok(Some(index));
                }
                continue;
            }
            Key::Down => {
                if selected + 1 < hits.len() || scanned < choices.len() {
                    selected += 1;
                }
                continue;
            }
            Key::Up => {
                selected = selected.saturating_sub(1);
                continue;
            }
            Key::Ignore => continue,
        }
        selected = 0;
        scanned = 0;
        hits.clear();
    }
}
