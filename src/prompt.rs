use std::ffi::CStr;

use crate::Session;

pub fn render(session: &mut Session, secondary: bool, status: u8, editing: bool) -> String {
    let name = if secondary { "PS2" } else { "PS1" };
    let Some(prompt) = session.get(name) else {
        return String::new();
    };
    render_mode(session, prompt, status, editing)
}

pub fn render_text(session: &mut Session, prompt: String, status: u8) -> String {
    render_mode(session, prompt, status, false)
}

fn render_mode(session: &mut Session, mut prompt: String, status: u8, editing: bool) -> String {
    let bytes = prompt.as_bytes();
    let mut escapes = memchr::memchr3_iter(b'\\', b'$', b'`', bytes);
    let first = escapes.next();
    let posix_bang = session.posix && bytes.contains(&b'!');
    if first.is_none() && !posix_bang {
        return prompt;
    }
    // An already-rendered ANSI prompt often contains only Readline width markers.
    // Strip those literal pairs without allocating shell words or expanding anything.
    let marker = |i: usize| bytes[i] == b'\\' && matches!(bytes.get(i + 1), Some(b'[' | b']'));
    if !posix_bang && first.is_some_and(marker) && escapes.all(marker) {
        if editing {
            return prompt.replace(r"\[", "\x01").replace(r"\]", "\x02");
        }
        let mut bracket = false;
        prompt.retain(|c| {
            if bracket {
                bracket = false;
                false
            } else {
                bracket = c == '\\';
                !bracket
            }
        });
        return prompt;
    }
    let parsed = crate::parser::prompt_word(
        &prompt,
        session.shopt.promptvars || session.posix,
        session.posix,
        |c, chars, output| match c {
            '[' if editing => output.push('\x01'),
            ']' if editing => output.push('\x02'),
            _ => escape(session, c, chars, output),
        },
    );
    parsed
        .ok()
        .and_then(|word| crate::eval::expand_case(session, word, status, false).ok())
        .unwrap_or(prompt)
}

fn escape(session: &Session, c: char, chars: &mut std::str::Chars, output: &mut String) {
    use std::fmt::Write;
    match c {
        '[' | ']' => {}
        'e' => output.push('\x1b'),
        'n' => output.push('\n'),
        'r' => output.push('\r'),
        'a' => output.push('\x07'),
        '\\' => output.push('\\'),
        '$' => output.push(if unsafe { libc::geteuid() } == 0 {
            '#'
        } else {
            '$'
        }),
        's' => output.push_str(
            session
                .arg0
                .rsplit('/')
                .next()
                .unwrap_or("tonari")
                .trim_start_matches('-'),
        ),
        'v' => output.push_str(crate::session::BASH_SERIES),
        'V' => output.push_str(crate::session::BASH_RELEASE),
        '!' => {
            let _ = write!(output, "{}", session.history_number());
        }
        '#' => {
            let _ = write!(output, "{}", session.command_number + 1);
        }
        'j' => {
            let _ = write!(output, "{}", session.jobs.count());
        }
        'l' => {
            let mut buf = [0u8; 256];
            if unsafe { libc::ttyname_r(0, buf.as_mut_ptr().cast(), buf.len()) } == 0 {
                let tty = unsafe { CStr::from_ptr(buf.as_ptr().cast()) }.to_string_lossy();
                output.push_str(tty.rsplit('/').next().unwrap_or("tty"));
            } else {
                output.push_str("tty");
            }
        }
        'd' | 't' | 'T' | '@' | 'A' | 'D' => {
            let fmt = match c {
                'd' => "%a %b %d",
                't' => "%H:%M:%S",
                'T' => "%I:%M:%S",
                '@' => "%I:%M %p",
                'A' => "%H:%M",
                _ => {
                    if let Some(rest) = chars.as_str().strip_prefix('{')
                        && let Some((fmt, tail)) = rest.split_once('}')
                    {
                        *chars = tail.chars();
                        if fmt.is_empty() { "%X" } else { fmt }
                    } else {
                        output.push_str("\\D");
                        return;
                    }
                }
            };
            output.push_str(&crate::shell_time::format(session, fmt, unsafe {
                libc::time(std::ptr::null_mut())
            }));
        }
        '0'..='7' => {
            let octet = |c: char, chars: &mut std::str::Chars<'_>| {
                crate::word::quote::char_escape_digits(chars, 8, 2, c.to_digit(8).unwrap()).0 as u8
            };
            let rest = chars.as_str();
            let first = octet(c, chars);
            if first == 0 {
                output.push('\\');
                output.push(c);
                output.push_str(&rest[..rest.len() - chars.as_str().len()]);
                return;
            }
            if first.is_ascii() {
                output.push(first as char);
            } else {
                let mut bytes = vec![first];
                loop {
                    let mut next = chars.clone();
                    if next.next() != Some('\\') {
                        break;
                    }
                    let Some(octal @ '0'..='7') = next.next() else {
                        break;
                    };
                    bytes.push(octet(octal, &mut next));
                    *chars = next;
                }
                output.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        'u' => {
            if let Some(user) = crate::word::effective_user() {
                output.push_str(user);
            }
        }
        c @ ('h' | 'H') => {
            if let Some(host) = crate::session::hostname() {
                output.push_str(if c == 'h' {
                    host.split('.').next().unwrap_or("")
                } else {
                    host
                });
            }
        }
        c @ ('w' | 'W') => {
            let start = output.len();
            let path = session
                .scalar_setting("PWD")
                .or_else(|| {
                    session
                        .logical_directory_ref()
                        .ok()?
                        .to_str()
                        .map(std::borrow::Cow::Borrowed)
                })
                .unwrap_or_default();
            let home = session
                .scalar_setting("HOME")
                .filter(|h| !h.is_empty() && h.as_ref() != "/");
            if home.as_ref() == Some(&path) {
                output.push('~');
            } else if c == 'W' {
                output.push_str(
                    path.trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("/"),
                );
            } else if let Some(rest) = home
                .as_ref()
                .and_then(|h| path.strip_prefix(h.as_ref()))
                .filter(|s| s.starts_with('/'))
            {
                output.push('~');
                output.push_str(rest);
            } else {
                output.push_str(&path);
            }
            if c == 'w'
                && let Some(limit) = session
                    .scalar_setting("PROMPT_DIRTRIM")
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|n| *n > 0)
            {
                let pwd = &output[start..];
                let prefix = if pwd.starts_with("~/") { "~/" } else { "" };
                let path = pwd.strip_prefix("~/").unwrap_or(pwd);
                if let Some((i, _)) = path.rmatch_indices('/').nth(limit - 1)
                    && i > 3
                {
                    let short = format!("{prefix}.../{}", &path[i + 1..]);
                    output.truncate(start);
                    output.push_str(&short);
                }
            }
        }
        c => {
            output.push('\\');
            output.push(c);
        }
    }
}
