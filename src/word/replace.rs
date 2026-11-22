use super::modifier::ReplaceMode;

fn write_replacement(out: &mut String, template: &str, matched: &str, expand_amp: bool) {
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push(chars.next().unwrap_or(c));
        } else if c == '&' && expand_amp {
            out.push_str(matched);
        } else {
            out.push(c);
        }
    }
}

pub(super) fn substitute(
    mut text: String,
    pattern: &str,
    template: &str,
    mode: ReplaceMode,
    expand_amp: bool,
) -> String {
    use ReplaceMode::*;
    if !crate::pattern::has_magic(pattern) {
        let needle = crate::pattern::unescape(pattern);
        let mut replacement = String::new();
        write_replacement(&mut replacement, template, &needle, expand_amp);
        if needle.is_empty() && !matches!(mode, Prefix | Suffix) {
            return text;
        }
        return match mode {
            First => text.replacen(&needle, &replacement, 1),
            All => text.replace(&needle, &replacement),
            Prefix if text.starts_with(&needle) => {
                text.replace_range(..needle.len(), &replacement);
                text
            }
            Suffix if text.ends_with(&needle) => {
                text.replace_range(text.len() - needle.len().., &replacement);
                text
            }
            _ => text,
        };
    }
    if pattern == "*" {
        let mut out = String::new();
        write_replacement(&mut out, template, &text, expand_amp);
        return out;
    }
    let bounds: Vec<_> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut out = String::new();
    let mut copied = 0;
    let mut i = 0;
    let starts = format!("{pattern}*");
    while i + 1 < bounds.len() {
        let start = bounds[i];
        let end = if pattern == "?" {
            (mode != Suffix || i + 2 == bounds.len()).then_some(i + 1)
        } else if mode == Suffix {
            crate::pattern::matches(&text[start..], pattern).then_some(bounds.len() - 1)
        } else if crate::pattern::matches(&text[start..], &starts) {
            (i + 1..bounds.len())
                .rev()
                .find(|&j| crate::pattern::matches(&text[start..bounds[j]], pattern))
        } else {
            None
        };
        if let Some(j) = end {
            out.push_str(&text[copied..start]);
            write_replacement(&mut out, template, &text[start..bounds[j]], expand_amp);
            copied = bounds[j];
            i = j;
            if mode != All {
                break;
            }
        } else {
            if mode == Prefix {
                break;
            }
            i += 1;
        }
    }
    if copied == 0 {
        return text;
    }
    out.push_str(&text[copied..]);
    out
}
