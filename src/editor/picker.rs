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

fn draw(screen: &mut Screen, choices: &[String], hits: &[usize], selected: usize, query: &str) {
    let rows = screen.height.saturating_sub(1).clamp(1, 8);
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
            let mut sequence = Vec::new();
            loop {
                if keys.queued.is_empty() && !ready(0, session.bindings.timeout().min(50)) {
                    break;
                }
                let Some(b) = keys.get()? else { break };
                sequence.push(b);
                match sequence.as_slice() {
                    b"[A" | b"OA" | b"[Z" => return Ok(Key::Up),
                    b"[B" | b"OB" => return Ok(Key::Down),
                    b"[200~" => return Ok(Key::Paste),
                    b"O" | b"[" | b"[2" | b"[20" | b"[200" => {}
                    _ => break,
                }
            }
            // Esc followed immediately by ordinary text cancels, then types it.
            if !sequence.starts_with(b"[") && !sequence.starts_with(b"O") {
                for b in sequence.into_iter().rev() {
                    keys.queued.push_front(b);
                }
            }
            Key::Cancel
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
    let mut hits: Vec<usize> = (0..choices.len()).collect();
    let mut selected = 0;
    loop {
        draw(screen, choices, &hits, selected, &query);
        io::stderr().write_all(screen.output.as_bytes())?;
        let Some(first) = keys.get()? else {
            return Ok(None);
        };
        match read_key(first, keys, session)? {
            Key::Cancel => return Ok(None),
            Key::Accept => {
                if let Some(&index) = hits.get(selected) {
                    return Ok(Some(index));
                }
            }
            Key::Down => {
                selected = (selected + 1).min(hits.len().saturating_sub(1));
            }
            Key::Up => {
                selected = selected.saturating_sub(1);
            }
            key => {
                match key {
                    Key::Text => {
                        keys.insert_burst(first, session, &mut inserted)?;
                        query.push_str(&inserted);
                    }
                    Key::Paste => query.push_str(&keys.paste(|_| Ok(()))?),
                    Key::Erase => {
                        query.pop();
                    }
                    _ => unreachable!(),
                }
                selected = 0;
                hits.clear();
                hits.extend((0..choices.len()).filter(|&i| choices[i].contains(&query)));
            }
        }
    }
}
