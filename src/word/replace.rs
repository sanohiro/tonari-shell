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
    extglob: bool,
    fold: Option<crate::session::CharacterLocale>,
) -> String {
    use ReplaceMode::*;
    if let Some(locale) = fold
        && !pattern.is_empty()
        && !crate::pattern::has_magic(pattern, false)
        && !(extglob && crate::pattern::extended::has_group(pattern))
    {
        let needle = crate::pattern::unescape(pattern);
        let mut out = String::new();
        let mut copied = 0;
        for (start, _) in text.char_indices() {
            if start < copied {
                continue;
            }
            if mode == Prefix && start != 0 {
                break;
            }
            if let Some(n) = crate::pattern::folded_prefix(&text[start..], &needle, locale)
                && (mode != Suffix || start + n == text.len())
            {
                out.push_str(&text[copied..start]);
                copied = start + n;
                write_replacement(&mut out, template, &text[start..copied], expand_amp);
                if mode != All {
                    break;
                }
            }
        }
        if copied == 0 {
            return text;
        }
        out.push_str(&text[copied..]);
        return out;
    }
    let group = match fold {
        Some(locale) if !pattern.is_empty() => Some(
            crate::pattern::extended::Pattern::insensitive(pattern, extglob, locale),
        ),
        _ if extglob => crate::pattern::extended::Pattern::new(pattern),
        _ => None,
    };
    if group.is_none() && !crate::pattern::has_magic(pattern, false) {
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
    let accepts = |candidate| {
        group.as_ref().map_or_else(
            || crate::pattern::matches(candidate, pattern),
            |g| g.accepts(candidate, false),
        )
    };
    while i + 1 < bounds.len() {
        let start = bounds[i];
        let end = if pattern == "?" {
            (mode != Suffix || i + 2 == bounds.len()).then_some(i + 1)
        } else if mode == Suffix {
            accepts(&text[start..]).then_some(bounds.len() - 1)
        } else if group.is_some() || crate::pattern::matches(&text[start..], &starts) {
            (i..bounds.len())
                .rev()
                .find(|&j| accepts(&text[start..bounds[j]]))
        } else {
            None
        };
        if let Some(j) = end {
            out.push_str(&text[copied..start]);
            write_replacement(&mut out, template, &text[start..bounds[j]], expand_amp);
            copied = bounds[j];
            i = if i == j { i + 1 } else { j };
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
    if copied == 0 && out.is_empty() {
        let insert_at_end = if mode == Suffix && accepts("") {
            let anchored = format!("*{pattern}");
            match fold {
                Some(locale) => {
                    crate::pattern::extended::insensitive_match(&text, &anchored, extglob, locale)
                }
                None if extglob => crate::pattern::extended::matches(&text, &anchored),
                None => crate::pattern::matches(&text, &anchored),
            }
        } else {
            text.is_empty() && pattern.starts_with('*') && accepts("")
        };
        if insert_at_end {
            write_replacement(&mut text, template, "", expand_amp);
        }
        return text;
    }
    out.push_str(&text[copied..]);
    out
}
