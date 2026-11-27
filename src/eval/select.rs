use crate::{Flow, Session, parser::Statement};
use std::fmt::Write;
use unicode_width::UnicodeWidthStr;

fn menu(items: &[String], columns: usize) -> String {
    let digits = |n: usize| n.ilog10() as usize + 1;
    let width = items.iter().map(|s| s.width()).max().unwrap_or(0) + digits(items.len()) + 4;
    let rows = items.len().div_ceil((columns / width).max(1));
    let rows = if rows == 1 { items.len() } else { rows };
    let mut out = String::new();
    for row in 0..rows {
        let mut column = 0;
        for i in (row..items.len()).step_by(rows) {
            let pad = digits(if column == 0 { rows } else { items.len() });
            let _ = write!(out, "{:pad$}) {}", i + 1, items[i]);
            if i + rows < items.len() {
                let mut pos = column + pad + 2 + items[i].width();
                column += width;
                while pos < column {
                    if pos / 8 < column / 8 {
                        out.push('\t');
                        pos += 8 - pos % 8;
                    } else {
                        out.push(' ');
                        pos += 1;
                    }
                }
            }
        }
        out.push('\n');
    }
    out
}

pub(super) fn run(
    session: &mut Session,
    name: &str,
    items: &[String],
    body: &[Statement],
    status: u8,
) -> Flow {
    if items.is_empty() {
        return Flow::Continue(0);
    }
    let mut first = true;
    super::repeat(session, body, status, |s, last| {
        let mut show = first || s.get("REPLY").is_some_and(|s| s.is_empty());
        first = false;
        loop {
            let prompt = s.get("PS3");
            let prompt = prompt.as_deref().unwrap_or("#? ");
            if show {
                let columns = s
                    .get("COLUMNS")
                    .and_then(|v| v.parse().ok())
                    .filter(|&n| n != 0)
                    .unwrap_or(80);
                let mut display = menu(items, columns);
                display.push_str(prompt);
                eprint!("{display}");
            } else {
                eprint!("{prompt}");
            }
            match s.read(&[], None) {
                Flow::Continue(0) => {}
                Flow::Continue(_) => {
                    println!();
                    return Err(Flow::Continue(1));
                }
                flow => return Err(flow),
            }
            let Some(reply) = s.get("REPLY") else {
                return Err(Flow::Continue(1));
            };
            if reply.is_empty() {
                show = true;
                continue;
            }
            let item = reply
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|n| items.get(n));
            if s.readonly(name) {
                diagnostic!("{name}: readonly variable");
                return Err(Flow::Continue(1));
            }
            if let Err(error) = s.assign_target(name, item.map_or("", String::as_str)) {
                diagnostic!("{name}: {error}");
                return Err(Flow::Discard(1));
            }
            return Ok(Some(last));
        }
    })
}
