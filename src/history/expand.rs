use crate::Session;
use std::{borrow::Cow, io};

#[derive(Clone)]
struct CachedWords {
    line: String,
    extglob: bool,
    spans: Vec<(usize, usize)>,
}

#[derive(Default, Clone)]
pub(crate) struct Memory {
    substitution: Option<(String, String)>,
    search: String,
    matched_word: Option<String>,
    words: Option<CachedWords>,
}

impl Memory {
    fn words(&mut self, line: &str, extglob: bool) -> &[(usize, usize)] {
        if self
            .words
            .as_ref()
            .is_none_or(|saved| saved.extglob != extglob || saved.line != line)
        {
            self.words = Some(CachedWords {
                line: line.to_owned(),
                extglob,
                spans: crate::parser::history_designator_spans(line, extglob),
            });
        }
        &self.words.as_ref().unwrap().spans
    }
}

pub(crate) struct Expansion<'a> {
    pub text: Cow<'a, str>,
    pub print: bool,
}
fn error(s: &str) -> io::Error {
    io::Error::other(s)
}

fn select(s: &str, designator: &str, words: &[(usize, usize)]) -> io::Result<String> {
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
    let mut chosen = String::with_capacity(words[hi].1 - words[lo].0);
    chosen.push_str(&s[words[lo].0..words[lo].1]);
    for &(start, end) in &words[lo + 1..hi + 1] {
        chosen.push(' ');
        chosen.push_str(&s[start..end]);
    }
    Ok(chosen)
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
use crate::word::quote::single_quote as shell_quote;

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
    expand_line(session, line, None)
}

fn quick_substitution(
    session: &mut Session,
    q: char,
    mut rest: &str,
) -> io::Result<Expansion<'static>> {
    let old = field(&mut rest, q);
    let new = field(&mut rest, q);
    let previous = session
        .history
        .as_deref()
        .and_then(|h| h.lines.last())
        .ok_or_else(|| error("event not found"))?;
    let memory = session
        .history_expansion
        .get_or_insert_with(Default::default);
    remember(memory, old, new)?;
    Ok(Expansion {
        text: Cow::Owned(
            substitute(previous, memory, false)? + rest.strip_prefix(":p").unwrap_or(rest),
        ),
        print: rest.starts_with(":p"),
    })
}

pub(crate) fn expand_line<'a>(
    session: &mut Session,
    line: &'a str,
    mut quote: Option<char>,
) -> io::Result<Expansion<'a>> {
    let chars = session.scalar_setting("histchars");
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
        && let Some(rest) = line.strip_prefix(q)
    {
        return quick_substitution(session, q, rest);
    }
    if !line.contains(event) {
        return Ok(unchanged());
    }
    let mut out = String::with_capacity(line.len());
    let mut input = line;
    let mut print = false;
    let mut expanded = false;
    let mut substitutions = Vec::new();
    while let Some(c) = pop(&mut input) {
        if quote == Some('$') {
            out.push(c);
            if !matches!(c, '\\' | '\'') {
                let end = memchr::memchr2(b'\\', b'\'', input.as_bytes()).unwrap_or(input.len());
                out.push_str(&input[..end]);
                input = &input[end..];
                continue;
            }
            if c == '\\'
                && let Some(next) = pop(&mut input)
            {
                out.push(next);
            } else if c == '\'' {
                quote = None;
            }
            continue;
        }
        if c == '$' && quote.is_none() && input.starts_with('\'') {
            input = &input[1..];
            out.push_str("$'");
            quote = Some('$');
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            out.push(c);
            if let Some(next) = pop(&mut input) {
                out.push(next);
            }
            continue;
        }
        if c == '$' && quote != Some('\'') && input.starts_with('(') {
            out.push_str("$(");
            input = &input[1..];
            substitutions.push((quote.take(), 0usize));
            continue;
        }
        if quote.is_none()
            && let Some((_, depth)) = substitutions.last_mut()
        {
            if c == '(' {
                *depth += 1;
            } else if c == ')' {
                if *depth == 0 {
                    quote = substitutions.pop().unwrap().0;
                } else {
                    *depth -= 1;
                }
                out.push(c);
                continue;
            }
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
            || quote == Some('"') && (session.posix || input.starts_with('"'))
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
            let needle = if search.is_empty() {
                memory.search.as_str()
            } else {
                search
            };
            if needle.is_empty() {
                return Err(error("no previous search"));
            }
            let result = hist
                .iter()
                .rev()
                .find_map(|s| s.rfind(needle).map(|at| (s.as_str(), at)));
            if let Some((entry, at)) = result {
                if !search.is_empty() && search != memory.search {
                    search.clone_into(&mut memory.search);
                }
                memory.matched_word = memory
                    .words(entry, session.shopt.glob.extglob)
                    .iter()
                    .copied()
                    .find(|&(start, end)| start <= at && at < end)
                    .map(|(start, end)| &entry[start..end])
                    .map(str::to_owned);
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
            let original = input;
            if input.starts_with('-') {
                pop(&mut input);
            }
            take(&mut input, |c| {
                Some(c) != quote
                    && !c.is_whitespace()
                    && !matches!(
                        c,
                        ':' | ';' | '|' | '&' | '(' | ')' | '<' | '>' | '^' | '$' | '*' | '%' | '-'
                    )
            });
            let prefix = &original[..original.len() - input.len()];
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
            let spec = if input.starts_with(['$', '*', '%']) {
                let (spec, rest) = input.split_at(1);
                input = rest;
                spec
            } else {
                take(&mut input, |c| {
                    c.is_ascii_digit() || matches!(c, '-' | '$' | '*' | '^')
                })
            };
            value = if spec == "%" {
                session
                    .history_expansion
                    .as_deref()
                    .and_then(|m| m.matched_word.clone())
                    .ok_or_else(|| error("no matching word"))?
            } else {
                select(
                    &value,
                    spec,
                    session
                        .history_expansion
                        .get_or_insert_with(Default::default)
                        .words(&value, session.shopt.glob.extglob),
                )?
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
