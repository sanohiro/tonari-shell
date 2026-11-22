use std::io::{self, Write};

fn current() -> libc::mode_t {
    // The interpreter runs builtins on one thread, so no file creation can
    // interleave with reading and restoring the process-wide mask.
    unsafe {
        let mask = libc::umask(0o022);
        libc::umask(mask);
        mask
    }
}

fn symbolic(text: &str, mask: libc::mode_t) -> Option<libc::mode_t> {
    let mut allowed = !mask & 0o777;
    for clause in text.split(',') {
        let mut chars = clause.chars().peekable();
        let mut who = 0;
        while let Some(&c) = chars.peek() {
            who |= match c {
                'u' => 0o700,
                'g' => 0o070,
                'o' => 0o007,
                'a' => 0o777,
                _ => break,
            };
            chars.next();
        }
        if who == 0 {
            who = 0o777;
        }
        let operator = chars.next()?;
        let mut bits = 0;
        for c in chars {
            bits |= match c {
                'r' => 0o444,
                'w' => 0o222,
                'x' => 0o111,
                _ => return None,
            };
        }
        bits &= who;
        match operator {
            '+' => allowed |= bits,
            '-' => allowed &= !bits,
            '=' => allowed = (allowed & !who) | bits,
            _ => return None,
        }
    }
    Some(!allowed & 0o777)
}

pub fn run(args: &[String]) -> u8 {
    let mut args = args.iter().peekable();
    let mut symbols = false;
    let mut reusable = false;
    while let Some(arg) = args.next_if(|arg| arg.starts_with('-') && arg.len() > 1) {
        if arg == "--" {
            break;
        }
        for flag in arg[1..].chars() {
            match flag {
                'S' => symbols = true,
                'p' => reusable = true,
                _ => {
                    eprintln!("tonari: umask: invalid option: {flag}");
                    return 2;
                }
            }
        }
    }
    let mut out = io::stdout().lock();
    let mask = if let Some(text) = args.next() {
        let mask = if text.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            libc::mode_t::from_str_radix(text, 8)
                .ok()
                .filter(|&n| n <= 0o7777)
        } else {
            symbolic(text, current())
        };
        let Some(mask) = mask else {
            eprintln!("tonari: umask: invalid mode: {text}");
            return 1;
        };
        unsafe { libc::umask(mask) };
        if !symbols {
            return 0;
        }
        mask
    } else {
        if reusable && write!(out, "umask {}", if symbols { "-S " } else { "" }).is_err() {
            return 1;
        }
        current()
    };
    let result = if symbols {
        let mut text = String::with_capacity(19);
        for (prefix, shift) in [("u=", 6), (",g=", 3), (",o=", 0)] {
            text.push_str(prefix);
            for (ch, bit) in [('r', 4), ('w', 2), ('x', 1)] {
                if (mask >> shift) & bit == 0 {
                    text.push(ch);
                }
            }
        }
        writeln!(out, "{text}")
    } else {
        writeln!(out, "{mask:04o}")
    };
    u8::from(result.and_then(|_| out.flush()).is_err())
}
