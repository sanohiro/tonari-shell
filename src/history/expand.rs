use crate::Session;
use std::{borrow::Cow, io};

pub(crate) struct Expansion<'a> {
    pub text: Cow<'a, str>,
    pub print: bool,
}
fn error(s: &str) -> io::Error {
    io::Error::other(s)
}

fn select(s: &str, designator: &str, matched: Option<&str>) -> io::Result<String> {
    let words = crate::parser::history_word_spans(s);
    if designator == "%" {
        return matched
            .and_then(|m| words.iter().map(|&(a, b)| &s[a..b]).find(|w| w.contains(m)))
            .map(str::to_owned)
            .ok_or_else(|| error("no matching word"));
    }
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
    Ok(s[words[lo].0..words[hi].1].to_owned())
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
    while let Some(c) = pop(s) {
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

pub(crate) fn expand<'a>(session: &Session, line: &'a str) -> io::Result<Expansion<'a>> {
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
        if old.is_empty() || !previous.contains(&old) {
            return Err(error("substitution failed"));
        }
        return Ok(Expansion {
            text: Cow::Owned(previous.replacen(&old, &new, 1) + rest),
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
            || input.starts_with(['=', '('])
            || quote == Some('"') && input.starts_with('"')
            || out.ends_with("${")
        {
            out.push(c);
            continue;
        }
        let hist = history.map_or(&[][..], |h| h.lines.as_slice());
        let prev = || hist.last().map(String::as_str);
        let mut matched = None;
        let value = if input.starts_with(event) {
            pop(&mut input);
            prev()
        } else if let Some(tail) = input.strip_prefix('#') {
            input = tail;
            Some(out.as_str())
        } else if input.starts_with(['^', '$', '*', ':', '%']) {
            prev()
        } else if let Some(tail) = input.strip_prefix('?') {
            input = tail;
            let search = take(&mut input, |c| c != '?' && c != '\n');
            if input.starts_with('?') {
                pop(&mut input);
            }
            matched = Some(search);
            hist.iter()
                .rev()
                .find(|s| s.contains(search))
                .map(String::as_str)
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
                    && !matches!(c, ':' | '=' | ';' | '|' | '&' | '(' | ')' | '"' | '\'')
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
        if candidate.starts_with(['^', '$', '*', '%'])
            || colon && candidate.starts_with(|c: char| c == '-' || c.is_ascii_digit())
        {
            input = candidate;
            let spec = take(&mut input, |c| {
                c.is_ascii_digit() || matches!(c, '-' | '$' | '*' | '%' | '^')
            });
            value = select(&value, spec, matched)?;
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
                    let dot = value.rfind('.').filter(|i| !value[*i..].contains('/'));
                    match (code, dot) {
                        ('r', Some(i)) => value[..i].into(),
                        ('e', Some(i)) => value[i + 1..].into(),
                        ('e', _) => String::new(),
                        _ => value,
                    }
                }
                's' => {
                    let d =
                        pop(&mut input).ok_or_else(|| error("missing substitution delimiter"))?;
                    let old = field(&mut input, d);
                    let new = field(&mut input, d);
                    if old.is_empty() || !value.contains(&old) {
                        return Err(error("substitution failed"));
                    }
                    let new = new.replace('&', &old);
                    if global {
                        value.replace(&old, &new)
                    } else {
                        value.replacen(&old, &new, 1)
                    }
                }
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
