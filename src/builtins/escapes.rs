// Bash echo -e, printf's format, and printf %b have distinct octal and \\c rules.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    Echo,
    Format,
    Argument,
}

pub(crate) fn decode(mut input: &[u8], out: &mut Vec<u8>, mode: Mode) -> bool {
    while let Some((&byte, rest)) = input.split_first() {
        input = rest;
        if byte == b'\\' {
            if !one(&mut input, out, mode) {
                return false;
            }
        } else {
            out.push(byte);
        }
    }
    true
}

pub(crate) fn one(input: &mut &[u8], out: &mut Vec<u8>, mode: Mode) -> bool {
    let Some((&escape, rest)) = input.split_first() else {
        out.push(b'\\');
        return true;
    };
    *input = rest;
    match escape {
        b'a' => out.push(7),
        b'b' => out.push(8),
        b'e' | b'E' => out.push(27),
        b'f' => out.push(12),
        b'n' => out.push(b'\n'),
        b'r' => out.push(b'\r'),
        b't' => out.push(b'\t'),
        b'v' => out.push(11),
        b'\\' => out.push(b'\\'),
        b'"' | b'\'' | b'?' if mode == Mode::Format => out.push(escape),
        b'c' if mode != Mode::Format => return false,
        b'0'..=b'7' if mode != Mode::Echo || escape == b'0' => {
            let extra = if escape == b'0' && mode != Mode::Format {
                3
            } else {
                2
            };
            let (value, _) = digits(input, 8, extra, u32::from(escape - b'0'));
            out.push(value as u8);
        }
        b'x' | b'u' | b'U' => {
            let limit = match escape {
                b'x' => 2,
                b'u' => 4,
                _ => 8,
            };
            let (value, count) = digits(input, 16, limit, 0);
            if count == 0 {
                if mode != Mode::Echo {
                    diagnostic!("printf: missing hexadecimal digit for \\{}", escape as char);
                }
                out.extend_from_slice(&[b'\\', escape]);
            } else if escape == b'x' {
                out.push(value as u8);
            } else if let Some(c) = char::from_u32(value) {
                out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            } else {
                out.extend_from_slice(&[b'\\', escape]);
                out.extend_from_slice(&rest[..count]);
            }
        }
        _ => out.extend_from_slice(&[b'\\', escape]),
    }
    true
}

fn digits(input: &mut &[u8], radix: u32, limit: usize, initial: u32) -> (u32, usize) {
    let (value, count) = crate::word::quote::escape_digits(input, radix, limit, initial);
    *input = &input[count..];
    (value, count)
}
