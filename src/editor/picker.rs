// The picker owns only its query. The draft and its cursor stay in the editor
// until the user accepts a single, normally quoted completion.
use super::{Keys, Screen};
use crate::Session;
use std::fmt::Write as _;
use std::io::{self, Write as _};
use unicode_width::UnicodeWidthChar;

fn page_rows(screen: &Screen) -> usize {
    const MAX_ROWS: usize = 8;
    screen.height.saturating_sub(1).clamp(1, MAX_ROWS)
}

fn label(out: &mut String, text: &str, mut cells: usize) -> usize {
    let start = cells;
    for c in text.chars() {
        if c.is_control() {
            let escaped = super::control_glyph(c);
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

fn draw<'a>(
    screen: &mut Screen,
    heading: &str,
    page: impl Iterator<Item = &'a str>,
    selected_row: usize,
    query: &str,
    empty: &str,
) {
    let out = &mut screen.output;
    super::begin_redraw(out, screen.row);
    let prefix = label(out, heading, screen.width);
    let available = screen.width - prefix;
    let mut remaining = available;
    let mut start = query.len();
    for (i, c) in query.char_indices().rev() {
        let w = if c.is_control() {
            super::control_glyph(c).len()
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
    for (row, choice) in page.enumerate() {
        out.push_str("\r\n");
        let prefix = label(
            out,
            if row == selected_row { "> " } else { "  " },
            screen.width,
        );
        label(out, choice, screen.width - prefix);
        shown += 1;
    }
    if shown == 0 {
        out.push_str("\r\n");
        label(out, empty, screen.width);
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
    Changed,
    Up,
    Down,
    Accept,
    Cancel,
    Ignore,
}

fn read_key(first: u8, keys: &mut Keys, session: &Session, query: &mut String) -> io::Result<Key> {
    Ok(match first {
        b'\r' | b'\n' => Key::Accept,
        b'\t' | 14 => Key::Down,
        16 => Key::Up,
        7 => Key::Cancel,
        8 | 127 => {
            query.pop();
            Key::Changed
        }
        27 => {
            // Disambiguate a lone Escape from an arrow or bracketed paste.
            let timeout = session.bindings.timeout();
            if keys.queued.is_empty() && !super::input::key_ready(session, timeout.min(50))? {
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
                if keys.queued.is_empty() && !super::input::key_ready(session, timeout)? {
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
                            (b'~', 3) if introducer == b'[' && &params == b"200" => {
                                query.push_str(&keys.paste(|_| Ok(()))?);
                                Key::Changed
                            }
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
        32.. => {
            keys.append_burst(first, session, query)?;
            Key::Changed
        }
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
    let mut hits = Vec::new();
    let mut scanned = 0;
    let mut selected: usize = 0;
    let contains = |value: &str, needle: &str| {
        if !session.bindings.ignore_case { return value.contains(needle); }
        let locale = session.character_locale();
        value.char_indices().any(|(at,_)| crate::pattern::folded_prefix(&value[at..], needle, locale).is_some())
    };
    let rows = page_rows(screen);
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
                    if (!session.bindings.ignore_case && finder.find(choices[index].as_bytes()).is_some()) || (session.bindings.ignore_case && contains(&choices[index], &query)) {
                        hits.push(index);
                    }
                }
            }
        }
        selected = selected.min(hits.len().saturating_sub(1));
        let offset = selected / rows * rows;
        let page = hits
            .iter()
            .skip(offset)
            .take(rows)
            .map(|&i| choices[i].as_str());
        draw(
            screen,
            "filter> ",
            page,
            selected - offset,
            &query,
            "(no matches)",
        );
        io::stderr().write_all(screen.output.as_bytes())?;
        let Some(first) = keys.get()? else {
            return Ok(None);
        };
        let old_len = query.len();
        match read_key(first, keys, session, &mut query)? {
            Key::Changed => {}
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
        if query.len() > old_len {
            hits.retain(|&i| contains(&choices[i], &query));
        } else {
            scanned = 0;
            hits.clear();
        }
    }
}

pub(super) fn history(
    query: &mut String,
    history: &crate::history::History,
    session: &Session,
    keys: &mut Keys,
    screen: &mut Screen,
) -> io::Result<Option<String>> {
    let rows = page_rows(screen);
    let mut selected: usize = 0;
    let mut snapshot = None;
    let mut refresh = true;
    loop {
        let offset = selected / rows * rows;
        let wanted = offset + rows;
        if refresh {
            draw(
                screen,
                "history> ",
                std::iter::empty(),
                0,
                query,
                "(searching)",
            );
            io::stderr().write_all(screen.output.as_bytes())?;
        }
        let filter = snapshot.get_or_insert_with(|| crate::history::Filter::new(session, history));
        let waiting = || keys.input_pending();
        if refresh {
            filter.rewind(session);
            filter.fill(session, query, 1, waiting);
            if let Some(first) = filter.hits.first() {
                draw(
                    screen,
                    "history> ",
                    std::iter::once(first.as_ref()),
                    0,
                    query,
                    "",
                );
                io::stderr().write_all(screen.output.as_bytes())?;
            }
        }
        filter.fill(session, query, wanted, waiting);
        if filter.exhausted() {
            selected = selected.min(filter.hits.len().saturating_sub(1));
        }
        let offset = selected / rows * rows;
        let page = filter
            .hits
            .iter()
            .skip(offset)
            .take(rows)
            .map(AsRef::as_ref);
        draw(
            screen,
            "history> ",
            page,
            selected - offset,
            query,
            if filter.exhausted() {
                "(no matches)"
            } else {
                "(searching)"
            },
        );
        io::stderr().write_all(screen.output.as_bytes())?;
        refresh = false;
        let Some(first) = keys.get()? else {
            return Ok(None);
        };
        let key = if first == 18 {
            Key::Down
        } else {
            read_key(first, keys, session, query)?
        };
        match key {
            Key::Cancel => return Ok(None),
            Key::Accept => {
                filter.fill(session, query, selected + 1, || {
                    crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
                });
                selected = selected.min(filter.hits.len().saturating_sub(1));
                if selected < filter.hits.len() {
                    return Ok(Some(filter.hits[selected].to_string()));
                }
                continue;
            }
            Key::Up => {
                selected = selected.saturating_sub(1);
                continue;
            }
            Key::Down => {
                if !filter.exhausted() || selected + 1 < filter.hits.len() {
                    selected += 1;
                }
                continue;
            }
            Key::Ignore => continue,
            Key::Changed => {}
        }
        refresh = true;
        selected = 0;
    }
}
