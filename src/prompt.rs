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

pub fn render_text(session: &mut Session, prompt: String, status: u8) -> String {
    if !prompt.contains(['\\', '$', '`']) {
        return prompt;
    }
    let parsed = crate::parser::prompt_word(&prompt, |c| escape(session, c));
    parsed
        .ok()
        .and_then(|word| crate::eval::expand_case(session, word, status, false).ok())
        .unwrap_or(prompt)
}

fn escape(session: &Session, c: char) -> String {
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
        's' => output.push_str("tonari"),
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
        }
        c => {
            output.push('\\');
            output.push(c);
        }
    }
    output
}
