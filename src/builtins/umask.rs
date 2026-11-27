use std::io::Write;

thread_local! {
    // A fork inherits both the OS mask and this snapshot. All mask changes
    // in the interpreter go through the setter in this module.
    static MASK: std::cell::Cell<Option<libc::mode_t>> = const { std::cell::Cell::new(None) };
}

fn current() -> libc::mode_t {
    if let Some(mask) = MASK.get() {
        return mask;
    }
    // SAFETY: only the interpreter thread creates files or changes the mask.
    let mask = unsafe {
        let mask = libc::umask(0o022);
        libc::umask(mask);
        mask
    };
    MASK.set(Some(mask));
    mask
}

fn symbolic(text: &str, mask: libc::mode_t) -> Option<libc::mode_t> {
    let original = !mask & 0o777;
    let mut allowed = original;
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
        chars.peek()?;
        while let Some(operator) = chars.next() {
            let mut bits = 0;
            if let Some(&class @ ('u' | 'g' | 'o')) = chars.peek() {
                chars.next();
                let shift = match class {
                    'u' => 6,
                    'g' => 3,
                    _ => 0,
                };
                bits = ((allowed >> shift) & 7) * 0o111;
            } else {
                while let Some(c) =
                    chars.next_if(|c| matches!(c, 'r' | 'w' | 'x' | 'X' | 's' | 't'))
                {
                    bits |= match c {
                        'r' => 0o444,
                        'w' => 0o222,
                        'x' => 0o111,
                        'X' if original & 0o111 != 0 => 0o111,
                        _ => 0,
                    };
                }
            }
            bits &= who;
            match operator {
                '+' => allowed |= bits,
                '-' => allowed &= !bits,
                '=' => allowed = (allowed & !who) | bits,
                _ => return None,
            }
        }
    }
    Some(!allowed & 0o777)
}

pub fn run(args: &[String], posix: bool) -> u8 {
    let mut args = args;
    let mut symbols = false;
    let mut reusable = false;
    while let Some(arg) = crate::builtins::flags::next(&mut args) {
        for flag in arg[1..].chars() {
            match flag {
                'S' => symbols = true,
                'p' => reusable = true,
                _ => {
                    diagnostic!("umask: invalid option: {flag}");
                    return 2;
                }
            }
        }
    }
    let mask = if let Some(text) = args.first() {
        let mask = if text.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            libc::mode_t::from_str_radix(text, 8)
                .ok()
                .filter(|&n| n <= 0o7777)
        } else {
            symbolic(text, current())
        };
        let Some(mask) = mask else {
            diagnostic!("umask: invalid mode: {text}");
            return 1;
        };
        unsafe { libc::umask(mask) };
        MASK.set(Some(mask & 0o777));
        if !symbols || posix {
            return 0;
        }
        mask
    } else {
        current()
    };
    // The longest form is "umask -S u=rwx,g=rwx,o=rwx" plus a newline.
    let mut storage = [0; 32];
    let mut out = std::io::Cursor::new(&mut storage[..]);
    if reusable && args.is_empty() {
        out.write_all(if symbols { b"umask -S " } else { b"umask " })
            .unwrap();
    }
    if symbols {
        for (prefix, shift) in [(b"u=".as_slice(), 6), (b",g=", 3), (b",o=", 0)] {
            out.write_all(prefix).unwrap();
            for (ch, bit) in [(b'r', 4), (b'w', 2), (b'x', 1)] {
                if (mask >> shift) & bit == 0 {
                    out.write_all(&[ch]).unwrap();
                }
            }
        }
        out.write_all(b"\n").unwrap();
    } else {
        writeln!(out, "{mask:04o}").unwrap();
    }
    let len = out.position() as usize;
    crate::builtins::write_stdout_for("umask", &storage[..len])
}
