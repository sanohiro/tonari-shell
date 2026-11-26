use crate::Session;
use std::{borrow::Cow, io};

#[derive(Default, Clone)]
pub(crate) struct Memory {
    substitution: Option<(String, String)>,
    search: String,
    matched_word: Option<String>,
}

pub(crate) struct Expansion<'a> {
    pub text: Cow<'a, str>,
    pub print: bool,
}
fn error(s: &str) -> io::Error {
    io::Error::other(s)
}

fn select(s: &str, designator: &str) -> io::Result<String> {
    let words = crate::parser::history_designator_spans(s);
    if designator == "*" && words.len() < 2 {
        return Ok(String::new());
    }
    let last = words.len().saturating_sub(1);
    let index = |v: &str| match v {
        "^" => Ok(1),
        "$" => Ok(last),
        _ => v.parse::<usize>().map_err(|_| error("bad word specifier")),
    };
    let (lo, hi) = if designator == "*" {
        (1, last)
    } else if let Some(prefix) = designator.strip_suffix('*') {
        (index(prefix)?, last)
    } else if let Some((lo, hi)) = designator.split_once('-') {
        (
            if lo.is_empty() { 0 } else { index(lo)? },
            if hi.is_empty() {
                last.saturating_sub(1)
            } else {
                index(hi)?
            },
        )
    } else {
        let n = index(designator)?;
        (n, n)
    };
    if lo > hi || hi >= words.len() {
        return Err(error("bad word specifier"));
    }
    Ok(words[lo..=hi]
        .iter()
        .map(|&(a, b)| &s[a..b])
        .collect::<Vec<_>>()
        .join(" "))
}

fn take<'a>(s: &mut &'a str, f: impl Fn(char) -> bool) -> &'a str {
    let n = s.find(|c| !f(c)).unwrap_or(s.len());
    let (part, tail) = s.split_at(n);
    *s = tail;
    part
}
fn pop(s: &mut &str) -> Option<char> {
    let c = s.chars().next()?;
    *s = &s[c.len_utf8()..];
    Some(c)
}
fn field(s: &mut &str, delim: char) -> String {
    let mut out = String::new();
    while !s.starts_with('\n')
        && let Some(c) = pop(s)
    {
        if c == delim {
            break;
        }
        if c == '\\' && s.starts_with(delim) {
            pop(s);
            out.push(delim);
        } else {
            out.push(c);
        }
    }
    out
}
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn substitute(value: &str, memory: &Memory, global: bool) -> io::Result<String> {
    let Some((old, new)) = &memory.substitution else {
        return Err(error("no previous substitution"));
    };
    let mut matches = value.match_indices(old.as_str());
    let Some((mut at, _)) = matches.next() else {
        return Err(error("substitution failed"));
    };
    let mut result = String::with_capacity(value.len() - old.len() + new.len());
    let mut start = 0;
    loop {
        result.push_str(&value[start..at]);
        result.push_str(new);
        start = at + old.len();
        if !global {
            break;
        }
        let Some((next, _)) = matches.next() else {
            break;
        };
        at = next;
    }
    result.push_str(&value[start..]);
    Ok(result)
}

fn remember(memory: &mut Memory, mut old: String, new: String) -> io::Result<()> {
    if old.is_empty() {
        old = memory
            .substitution
            .as_ref()
            .map_or(&memory.search, |s| &s.0)
            .clone();
        if old.is_empty() {
            return Err(error("no previous substitution"));
        }
    }
    let replacement = if new.contains('&') {
        let mut result = String::with_capacity(new.len());
        let mut chars = new.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' && chars.next_if_eq(&'&').is_some() {
                result.push('&');
            } else if c == '&' {
                result.push_str(&old);
            } else {
                result.push(c);
            }
        }
        result
    } else {
        new
    };
    memory.substitution = Some((old, replacement));
    Ok(())
}

pub(crate) fn expand<'a>(session: &mut Session, line: &'a str) -> io::Result<Expansion<'a>> {
    let chars = session.get("histchars");
    let mut chars = chars.as_deref().unwrap_or("!^#").chars();
    let event = chars.next();
    let quick = chars.next();
    let comment = chars.next().unwrap_or('#');
    let unchanged = || Expansion {
        text: Cow::Borrowed(line),
        print: false,
    };
    let Some(event) = event else {
        return Ok(unchanged());
    };
    let history = session.history.as_deref();
    if let Some(q) = quick
        && let Some(mut rest) = line.strip_prefix(q)
    {
        let old = field(&mut rest, q);
        let new = field(&mut rest, q);
        let previous = history
            .and_then(|h| h.lines.last())
            .ok_or_else(|| error("event not found"))?;
        let memory = session
            .history_expansion
            .get_or_insert_with(Default::default);
        remember(memory, old, new)?;
        return Ok(Expansion {
            text: Cow::Owned(substitute(previous, memory, false)? + rest),
            print: false,
        });
    }
    if !line.contains(event) {
        return Ok(unchanged());
    }
    let mut out = String::with_capacity(line.len());
    let mut input = line;
    let mut quote = None;
    let mut print = false;
    let mut expanded = false;
    while let Some(c) = pop(&mut input) {
        if c == '\\' && quote != Some('\'') {
            out.push(c);
            if let Some(next) = pop(&mut input) {
                out.push(next);
            }
            continue;
        }
        if matches!(c, '\'' | '"') {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
            out.push(c);
            continue;
        }
        if c == comment && quote.is_none() && (out.is_empty() || out.ends_with(char::is_whitespace))
        {
            out.push(c);
            out.push_str(input);
            break;
        }
        if c != event
            || quote == Some('\'')
            || input.is_empty()
            || input.starts_with(char::is_whitespace)
            || input.starts_with(['=', ';', '&', '|', '(', ')', '<', '>'])
            || out.ends_with('[') && input.contains(']')
            || event == '!' && out.ends_with('$')
            || quote == Some('"') && input.starts_with('"')
            || out.ends_with("${")
        {
            out.push(c);
            continue;
        }
        let hist = history.map_or(&[][..], |h| h.lines.as_slice());
        let prev = || hist.last().map(String::as_str);
        let value = if input.starts_with(['^', '$', '*', '%']) {
            prev()
        } else if input.starts_with(event) {
            pop(&mut input);
            prev()
        } else if let Some(tail) = input.strip_prefix('#') {
            input = tail;
            Some(out.as_str())
        } else if input.starts_with(':') {
            prev()
        } else if let Some(tail) = input.strip_prefix('?') {
            input = tail;
            let search = take(&mut input, |c| c != '?' && c != '\n');
            if input.starts_with('?') {
                pop(&mut input);
            }
            let memory = session
                .history_expansion
                .get_or_insert_with(Default::default);
            let search = if search.is_empty() {
                memory.search.as_str()
            } else {
                search
            };
            if search.is_empty() {
                return Err(error("no previous search"));
            }
            let result = hist
                .iter()
                .rev()
                .find_map(|s| s.rfind(search).map(|at| (s.as_str(), at)));
            if let Some((entry, at)) = result {
                memory.matched_word = crate::parser::history_designator_spans(entry)
                    .into_iter()
                    .find(|&(start, end)| start <= at && at < end)
                    .map(|(start, end)| &entry[start..end])
                    .map(str::to_owned);
                if !search.is_empty() {
                    memory.search = search.to_owned();
                }
            }
            result.map(|(entry, _)| entry)
        } else if input.starts_with(|c: char| c.is_ascii_digit())
            || input
                .strip_prefix('-')
                .is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit()))
        {
            let negative = input.starts_with('-');
            if negative {
                pop(&mut input);
            }
            let n = take(&mut input, |c| c.is_ascii_digit())
                .parse::<usize>()
                .ok();
            n.and_then(|n| {
                if negative {
                    hist.len().checked_sub(n)
                } else {
                    n.checked_sub(history.map_or(0, |h| h.base) + 1)
                }
            })
            .and_then(|n| hist.get(n))
            .map(String::as_str)
        } else {
            let prefix = take(&mut input, |c| {
                !c.is_whitespace()
                    && !matches!(
                        c,
                        ':' | ';'
                            | '|'
                            | '&'
                            | '('
                            | ')'
                            | '<'
                            | '>'
                            | '"'
                            | '\''
                            | '^'
                            | '$'
                            | '*'
                            | '%'
                            | '-'
                    )
            });
            if prefix.is_empty() {
                out.push(event);
                continue;
            }
            hist.iter()
                .rev()
                .find(|s| s.starts_with(prefix))
                .map(String::as_str)
        };
        let mut value = value.ok_or_else(|| error("event not found"))?.to_owned();
        let colon = input.starts_with(':');
        let candidate = if colon { &input[1..] } else { input };
        if candidate.starts_with(['^', '$', '*', '%', '-'])
            || colon && candidate.starts_with(|c: char| c == '-' || c.is_ascii_digit())
        {
            input = candidate;
            let spec = take(&mut input, |c| {
                c.is_ascii_digit() || matches!(c, '-' | '$' | '*' | '%' | '^')
            });
            value = if spec == "%" {
                session
                    .history_expansion
                    .as_deref()
                    .and_then(|m| m.matched_word.clone())
                    .ok_or_else(|| error("no matching word"))?
            } else {
                select(&value, spec)?
            };
        }
        while input.starts_with(':') {
            pop(&mut input);
            let global = input.starts_with('g');
            if global {
                pop(&mut input);
            }
            let code = pop(&mut input).ok_or_else(|| error("missing history modifier"))?;
            value = match code {
                'p' => {
                    print = true;
                    value
                }
                'q' => shell_quote(&value),
                'x' => value
                    .split_whitespace()
                    .map(shell_quote)
                    .collect::<Vec<_>>()
                    .join(" "),
                't' => value.rsplit('/').next().unwrap_or("").into(),
                'h' => value
                    .rsplit_once('/')
                    .map_or(value.as_str(), |(h, _)| h)
                    .into(),
                'r' | 'e' => {
                    if let Some(i) = value.rfind('.') {
                        if code == 'r' {
                            value.truncate(i);
                        } else {
                            value.drain(..i);
                        }
                    }
                    value
                }
                's' => {
                    let d =
                        pop(&mut input).ok_or_else(|| error("missing substitution delimiter"))?;
                    let old = field(&mut input, d);
                    let new = field(&mut input, d);
                    let memory = session
                        .history_expansion
                        .get_or_insert_with(Default::default);
                    remember(memory, old, new)?;
                    substitute(&value, memory, global)?
                }
                '&' => substitute(
                    &value,
                    session
                        .history_expansion
                        .as_deref()
                        .ok_or_else(|| error("no previous substitution"))?,
                    global,
                )?,
                _ => return Err(error("unsupported history modifier")),
            };
        }
        expanded = true;
        out.push_str(&value);
    }
    Ok(if expanded {
        Expansion {
            text: Cow::Owned(out),
            print,
        }
    } else {
        unchanged()
    })
}
