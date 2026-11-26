use super::Error;
use std::str::Chars;

fn digits(chars: &mut Chars<'_>, radix: u32, count: usize) -> Option<u32> {
    let mut value = None;
    for _ in 0..count {
        let Some(n) = chars.clone().next().and_then(|c| c.to_digit(radix)) else {
            break;
        };
        chars.next();
        value = Some(value.unwrap_or(0) * radix + n);
    }
    value
}

/// Decode a dollar-single-quoted word. Hex and octal escapes produce bytes,
/// so adjacent escapes may together encode one UTF-8 character.
pub(super) fn quoted(chars: &mut Chars<'_>) -> Result<String, Error> {
    let source = chars.as_str();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                let end = source.len() - chars.as_str().len() - 1;
                return escapes(&source[..end], true);
            }
            '\\' => {
                chars.next();
            }
            _ => {}
        }
    }
    Err(Error::Incomplete(
        "unterminated dollar-single-quoted string",
    ))
}

pub(crate) fn decode(source: &str) -> Result<String, Error> {
    escapes(source, false)
}

fn escapes(source: &str, braced_hex: bool) -> Result<String, Error> {
    let mut bytes = Vec::with_capacity(source.len());
    let mut chars = source.chars();
    let mut buffer = [0u8; 4];
    while let Some(c) = chars.next() {
        if c != '\\' {
            bytes.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        let Some(escape) = chars.next() else {
            bytes.push(b'\\');
            break;
        };
        let byte = match escape {
            'a' => 7,
            'b' => 8,
            'e' | 'E' => 27,
            'f' => 12,
            'n' => 10,
            'r' => 13,
            't' => 9,
            'v' => 11,
            '\'' | '"' | '\\' | '?' => escape as u8,
            'c' if !chars.as_str().is_empty() => {
                let control = chars.next().unwrap();
                if control == '\\' && chars.as_str().starts_with('\\') {
                    chars.next();
                }
                if !control.is_ascii() {
                    return Err(Error::Invalid("control escape requires an ASCII character"));
                }
                if control == '?' {
                    127
                } else {
                    control as u8 & 31
                }
            }
            '0'..='7' => {
                let first = escape.to_digit(8).unwrap();
                let rest = chars.as_str();
                let tail = digits(&mut chars, 8, 2).unwrap_or(0);
                let count = rest.len() - chars.as_str().len();
                ((first << (3 * count)) + tail) as u8
            }
            'x' if braced_hex && chars.as_str().starts_with('{') => {
                chars.next();
                let mut byte = 0u8;
                while let Some(digit) = chars.clone().next().and_then(|c| c.to_digit(16)) {
                    byte = (byte << 4) | digit as u8;
                    chars.next();
                }
                if chars.as_str().starts_with('}') {
                    chars.next();
                }
                byte
            }
            'x' | 'u' | 'U' => {
                let count = match escape {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let Some(n) = digits(&mut chars, 16, count) else {
                    bytes.extend_from_slice(&[b'\\', escape as u8]);
                    continue;
                };
                if escape == 'x' || n == 0 {
                    n as u8
                } else {
                    let character =
                        char::from_u32(n).ok_or(Error::Invalid("invalid Unicode escape"))?;
                    bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
                    continue;
                }
            }
            _ => {
                bytes.push(b'\\');
                bytes.extend_from_slice(escape.encode_utf8(&mut buffer).as_bytes());
                continue;
            }
        };
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    String::from_utf8(bytes).map_err(|_| Error::Invalid("quoted bytes are not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_bytes_and_unicode_escapes() {
        assert_eq!(
            decode(r"\303\251\xC3\xA9\u00E9\U0001F469").unwrap(),
            "ééé👩"
        );
        assert_eq!(decode(r"\xq\z\cA\c?\c@ignored").unwrap(), "\\xq\\z\x01\x7f");
        assert!(decode(r"\x80").is_err());
    }
}
