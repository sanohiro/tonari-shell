pub(crate) fn single_quote(text: &str) -> String {
    let mut output = String::with_capacity(text.len() + 2);
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

pub(crate) fn needs_ansi(data: &[u8], ascii: bool) -> bool {
    data.iter()
        .any(|b| b.is_ascii_control() || ascii && !b.is_ascii())
}

pub(crate) fn ansi_quote(data: &[u8], ascii_locale: bool) -> Vec<u8> {
    let mut out = Vec::new();

    out.extend_from_slice(b"$'");
    for &b in data {
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
            b if b.is_ascii_control() || (ascii_locale && !b.is_ascii()) => {
                out.extend_from_slice(&[
                    b'\\',
                    b'0' + (b >> 6),
                    b'0' + ((b >> 3) & 7),
                    b'0' + (b & 7),
                ]);
            }
            _ => out.push(b),
        }
    }
    out.push(b'\'');

    out
}
