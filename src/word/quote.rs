pub(crate) fn single_quote(text: &str) -> String {
    let mut output = String::with_capacity(text.len() + 2);
    push_single_quote(&mut output, text);
    output
}

pub(crate) fn push_single_quote(output: &mut String, text: &str) {
    output.push('\'');
    let mut tail = text;
    while let Some((before, after)) = tail.split_once('\'') {
        output.push_str(before);
        output.push_str("'\\''");
        tail = after;
    }
    output.push_str(tail);
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
