use std::ffi::CStr;

use crate::Session;

pub fn render(session: &mut Session, secondary: bool, status: u8) -> String {
    let (name, fallback) = if secondary {
        ("PS2", "> ")
    } else {
        ("PS1", "tonari$ ")
    };
    let Some(prompt) = session.get(name) else {
        return fallback.to_string();
    };
    render_text(session, prompt, status)
}

pub fn render_text(session: &mut Session, mut prompt: String, status: u8) -> String {
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
        |c, chars| escape(session, c, chars),
    );
    parsed
        .ok()
        .and_then(|word| crate::eval::expand_case(session, word, status, false).ok())
        .unwrap_or(prompt)
}

fn escape(session: &Session, c: char, chars: &mut std::str::Chars) -> String {
    let mut output = String::new();
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
        'v' => output.push_str("5.2"),
        'V' => output.push_str("5.2.0"),
        '!' => output.push_str(&session.history_number().to_string()),
        '#' => output.push_str(&(session.command_number + 1).to_string()),
        'j' => output.push_str(&session.jobs.count().to_string()),
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
                        return "\\D".into();
                    }
                }
            };
            output.push_str(&crate::shell_time::format(session, fmt, unsafe {
                libc::time(std::ptr::null_mut())
            }));
        }
        '0'..='7' => {
            let mut n = c.to_digit(8).unwrap();
            for _ in 0..2 {
                if let Some(d) = chars.clone().next().and_then(|c| c.to_digit(8)) {
                    chars.next();
                    n = n * 8 + d;
                } else {
                    break;
                }
            }
            output.push(char::from(n as u8));
        }
        'u' => {
            let user = unsafe { libc::getpwuid(libc::geteuid()) };
            if !user.is_null() {
                output.push_str(&unsafe { CStr::from_ptr((*user).pw_name) }.to_string_lossy());
            }
        }
        c @ ('h' | 'H') => {
            let mut buf = [0u8; 256];
            if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) } == 0 {
                let host = String::from_utf8_lossy(
                    &buf[..buf.iter().position(|b| *b == 0).unwrap_or(buf.len())],
                );
                output.push_str(if c == 'h' {
                    host.split('.').next().unwrap_or("")
                } else {
                    &host
                });
            }
        }
        c @ ('w' | 'W') => {
            let path = session.get("PWD").unwrap_or_default();
            let home = session.get("HOME").filter(|h| !h.is_empty() && h != "/");
            if home.as_ref() == Some(&path) {
                output.push('~');
            } else if c == 'W' {
                output.push_str(
                    path.rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("/"),
                );
            } else if let Some(rest) = home
                .as_ref()
                .and_then(|h| path.strip_prefix(h))
                .filter(|s| s.starts_with('/'))
            {
                output.push('~');
                output.push_str(rest);
            } else {
                output.push_str(&path);
            }
            if c == 'w'
                && let Some(limit) = session
                    .get("PROMPT_DIRTRIM")
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|n| *n > 0)
            {
                let prefix = if output.starts_with("~/") { "~/" } else { "" };
                let path = output.trim_start_matches("~/").trim_start_matches('/');
                if let Some((i, _)) = path.rmatch_indices('/').nth(limit - 1)
                    && i > 3
                {
                    output = format!("{prefix}.../{}", &path[i + 1..]);
                }
            }
        }
        c => {
            output.push('\\');
            output.push(c);
        }
    }
    output
}
