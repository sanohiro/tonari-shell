//! Consume one short-option cluster. Each builtin decides which letters take
//! values and how to report invalid options. A lone dash remains an operand.
pub(crate) fn next<'a>(args: &mut &'a [String]) -> Option<&'a str> {
    let (word, rest) = args.split_first()?;
    if word.len() < 2 || !word.starts_with('-') {
        return None;
    }
    *args = rest;
    (word != "--").then_some(word)
}

pub(crate) fn value<'a>(attached: &'a str, args: &mut &'a [String]) -> Option<&'a str> {
    if attached.is_empty() {
        let (value, rest) = args.split_first()?;
        *args = rest;
        Some(value)
    } else {
        Some(attached)
    }
}
