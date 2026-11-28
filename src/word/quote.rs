// Escape digits are ASCII in shell words, prompt strings, echo and printf.
pub(crate) fn escape_digits(
    input: &[u8],
    radix: u32,
    limit: usize,
    mut value: u32,
) -> (u32, usize) {
    let mut count = 0;
    for &b in input.iter().take(limit) {
        let Some(digit) = (b as char).to_digit(radix) else {
            break;
        };
        value = value * radix + digit;
        count += 1;
    }
    (value, count)
}

pub(crate) fn char_escape_digits(
    chars: &mut std::str::Chars<'_>,
    radix: u32,
    limit: usize,
    initial: u32,
) -> (u32, usize) {
    let rest = chars.as_str();
    let (value, count) = escape_digits(rest.as_bytes(), radix, limit, initial);
    *chars = rest[count..].chars();
    (value, count)
}

pub(crate) fn single_quote(text: &str) -> String {
    let bytes = text.as_bytes();
    // For dense quotes, copying each ordinary byte is cheaper than restarting a search at each tiny span.
    if bytes.len() >= 256 && memchr::memchr_iter(b'\'', &bytes[..128]).take(8).count() == 8 {
        let count = memchr::memchr_iter(b'\'', bytes).count();
        let mut quoted = Vec::with_capacity(bytes.len() + 2 + 3 * count);
        quoted.push(b'\'');
        for &b in bytes {
            if b == b'\'' {
                quoted.extend_from_slice(b"'\\''");
            } else {
                quoted.push(b);
            }
        }
        quoted.push(b'\'');
        // Replacing an ASCII quote with ASCII preserves all UTF-8 boundaries.
        return unsafe { String::from_utf8_unchecked(quoted) };
    }
    let mut output = String::with_capacity(bytes.len() + 2);
    push_single_quote(&mut output, text);
    output
}

pub(crate) fn push_single_quote(output: &mut String, text: &str) {
    output.push('\'');
    let mut start = 0;
    for at in memchr::memchr_iter(b'\'', text.as_bytes()) {
        output.push_str(&text[start..at]);
        output.push_str("'\\''");
        start = at + 1;
    }
    output.push_str(&text[start..]);
    output.push('\'');
}

pub(crate) fn tilde_at(bytes: &[u8], i: usize) -> bool {
    bytes[i] == b'~' && (i == 0 || matches!(bytes[i - 1], b'=' | b':'))
}

pub(crate) fn shell_quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_./-".contains(&c))
    {
        text.to_owned()
    } else {
        single_quote(text)
    }
}

pub(crate) fn parameter_quote(text: &str, session: &crate::Session) -> String {
    let ascii = session.ascii_quoting();
    if needs_ansi(text.as_bytes(), ascii) {
        String::from_utf8(ansi_quote(text.as_bytes(), ascii)).expect("quoted UTF-8")
    } else {
        single_quote(text)
    }
}

fn escaped_scalar(c: char) -> bool {
    !c.is_ascii() && !crate::pattern::printable_utf8(c)
}

fn contains_nonprinting(data: &[u8]) -> bool {
    !data.is_ascii()
        && match std::str::from_utf8(data) {
            Ok(s) => s.chars().any(escaped_scalar),
            Err(_) => true,
        }
}

pub(crate) fn needs_ansi(data: &[u8], ascii: bool) -> bool {
    let (blocks, tail) = data.as_chunks::<8>();
    let ones = u64::from_ne_bytes([1; 8]);
    let high = ones << 7;
    let has_zero = |x: u64| x.wrapping_sub(ones) & !x & high != 0;
    blocks.iter().any(|block| {
        let word = u64::from_ne_bytes(*block);
        // A C0 control has its top three bits clear. DEL is exactly 0x7f.
        (ascii && word & high != 0)
            || has_zero(word & (ones * 0xe0))
            || has_zero(word ^ (ones * 0x7f))
    }) || tail
        .iter()
        .any(|b| b.is_ascii_control() || ascii && !b.is_ascii())
        || !ascii && contains_nonprinting(data)
}

pub(crate) fn ansi_quote(data: &[u8], ascii_locale: bool) -> Vec<u8> {
    let mut out = Vec::new();

    out.extend_from_slice(b"$'");
    let mut bytes = data;
    while let Some((&b, tail)) = bytes.split_first() {
        if !ascii_locale && b >= 0xc2 {
            // Inspect a single scalar; validating the remaining string for
            // every character would turn a Unicode listing into quadratic work.
            let width = if b < 0xe0 {
                2
            } else if b < 0xf0 {
                3
            } else {
                4
            };
            if let Some(scalar) = bytes.get(..width).and_then(|v| std::str::from_utf8(v).ok())
                && let Some(c) = scalar.chars().next()
            {
                if escaped_scalar(c) {
                    for &byte in scalar.as_bytes() {
                        push_octal(&mut out, byte);
                    }
                } else {
                    out.extend_from_slice(scalar.as_bytes());
                }
                bytes = &bytes[width..];
                continue;
            }
        }
        bytes = tail;
        match b {
            7 => out.extend_from_slice(b"\\a"),
            8 => out.extend_from_slice(b"\\b"),
            9 => out.extend_from_slice(b"\\t"),
            10 => out.extend_from_slice(b"\\n"),
            11 => out.extend_from_slice(b"\\v"),
            12 => out.extend_from_slice(b"\\f"),
            13 => out.extend_from_slice(b"\\r"),
            27 => out.extend_from_slice(b"\\E"),
            b'\'' | b'\\' => out.extend_from_slice(&[b'\\', b]),
            b if b.is_ascii_control() || !b.is_ascii() => {
                push_octal(&mut out, b);
            }
            _ => out.push(b),
        }
    }
    out.push(b'\'');

    out
}

fn push_octal(out: &mut Vec<u8>, b: u8) {
    out.extend_from_slice(&[
        b'\\',
        b'0' + (b >> 6),
        b'0' + ((b >> 3) & 7),
        b'0' + (b & 7),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_control_and_non_ascii_bytes_at_every_alignment() {
        for ascii in [false, true] {
            for i in 0..17 {
                let mut bytes = [b'A'; 17];
                for b in 0..=u8::MAX {
                    bytes[i] = b;
                    assert_eq!(
                        needs_ansi(&bytes, ascii),
                        b.is_ascii_control() || ascii && !b.is_ascii()
                    );
                }
            }
        }
    }
}
