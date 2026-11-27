use super::modifier::ReplaceMode;

fn write_replacement(out: &mut String, template: &str, matched: &str, expand_amp: bool) {
    if !expand_amp {
        out.push_str(template);
        return;
    }
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.peek(), Some('\\' | '&')) {
            out.push(chars.next().unwrap());
        } else if c == '&' {
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
    let expand_amp = expand_amp && template.contains(['\\', '&']);
    if expand_amp && template == "&" {
        return text;
    }
    if pattern == "?" {
        if mode == All {
            if template.is_empty() {
                text.clear();
                return text;
            }
            if !expand_amp {
                return template.repeat(text.chars().count());
            }
            let mut out = String::with_capacity(text.len());
            for (i, c) in text.char_indices() {
                write_replacement(&mut out, template, &text[i..i + c.len_utf8()], expand_amp);
            }
            return out;
        }
        let mut chars = text.char_indices();
        let selected = if mode == Suffix {
            chars.next_back()
        } else {
            chars.next()
        };
        if let Some((i, c)) = selected {
            let range = i..i + c.len_utf8();
            if expand_amp {
                let mut rendered = String::new();
                write_replacement(&mut rendered, template, &text[range.clone()], true);
                text.replace_range(range, &rendered);
            } else {
                text.replace_range(range, template);
            }
        }
        return text;
    }
    // A single bracket expression consumes one character. Avoid trying every
    // possible suffix when deleting matching characters from a string.
    if mode == All
        && fold.is_none()
        && let Some(body) = pattern.strip_prefix('[')
        && body.ends_with(']')
        && crate::pattern::bracket_end(body) == Some(body.len() - 1)
        && !body.contains("[.")
        && !body.contains("[=")
    {
        let matcher = crate::pattern::Matcher::new(pattern);
        let mut bytes = [0; 4];
        if template.is_empty() {
            text.retain(|c| !matcher.matches(c.encode_utf8(&mut bytes)));
            return text;
        }
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            let value = c.encode_utf8(&mut bytes);
            if matcher.matches(value) {
                write_replacement(&mut out, template, value, expand_amp);
            } else {
                out.push(c);
            }
        }
        return out;
    }
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
        let literal = crate::pattern::unescape(pattern);
        let needle = literal.as_ref();
        let mut rendered = String::new();
        let replacement = if expand_amp {
            write_replacement(&mut rendered, template, needle, expand_amp);
            rendered.as_str()
        } else {
            template
        };
        if needle.is_empty() && !matches!(mode, Prefix | Suffix) {
            return text;
        }
        return match mode {
            First => {
                if let Some(index) = text.find(needle) {
                    text.replace_range(index..index + needle.len(), replacement);
                }
                text
            }
            All => text.replace(needle, replacement),
            Prefix if text.starts_with(needle) => {
                text.replace_range(..needle.len(), replacement);
                text
            }
            Suffix if text.ends_with(needle) => {
                text.replace_range(text.len() - needle.len().., replacement);
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
    let mut copied = 0;
    let mut i = 0;
    let escaped_end = pattern.bytes().rev().take_while(|&b| b == b'\\').count() % 2 != 0;
    let basic = group
        .is_none()
        .then(|| crate::pattern::Matcher::new(pattern));
    let width = basic
        .as_ref()
        .and_then(|_| crate::pattern::FixedWidth::parse(pattern));
    let max_width = width.as_ref().map(|f| f.width);
    let ascii = width.as_ref().filter(|_| text.is_ascii());
    let mut out = String::new();
    if let Some(fixed) = ascii
        && mode == All
    {
        let width = fixed.width;
        debug_assert_ne!(width, 0);
        // With only edge wildcards, a single literal span decides a match.
        let literal = fixed.literal_span(pattern).filter(|_| !text.contains('\0'));
        while let Some(candidate) = text.get(i..i + width) {
            let matches = literal.map_or_else(
                || fixed.matches_ascii(pattern, candidate, basic.as_ref().unwrap()),
                |(offset, literal)| candidate.as_bytes()[offset..].starts_with(literal),
            );
            if matches {
                out.push_str(&text[copied..i]);
                write_replacement(&mut out, template, candidate, expand_amp);
                i += width;
                copied = i;
            } else {
                i += 1;
            }
        }
        if copied == 0 {
            return text;
        }
        out.push_str(&text[copied..]);
        return out;
    }
    let bounds: Vec<_> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let starts = (max_width.is_none() && group.is_none() && !escaped_end && mode != Suffix)
        .then(|| crate::pattern::Matcher::new(format!("{pattern}*")));
    let accepts = |candidate: &str| {
        if let Some(width) = ascii {
            return width.matches_ascii(pattern, candidate, basic.as_ref().unwrap());
        }
        group.as_ref().map_or_else(
            || basic.as_ref().unwrap().matches(candidate),
            |g| g.accepts(candidate, false),
        )
    };
    if mode == Suffix
        && let Some(width) = max_width
    {
        i = bounds.len().saturating_sub(width + 1);
    }
    while i + 1 < bounds.len() {
        let start = bounds[i];
        let end = if mode == Suffix {
            accepts(&text[start..]).then_some(bounds.len() - 1)
        } else if let Some(width) = max_width {
            let last = i.saturating_add(width);
            if ascii.is_some() {
                (last < bounds.len() && accepts(&text[start..bounds[last]])).then_some(last)
            } else {
                (i + 1..=last.min(bounds.len() - 1))
                    .rev()
                    .find(|&j| accepts(&text[start..bounds[j]]))
            }
        } else if starts
            .as_ref()
            .is_none_or(|matcher| matcher.matches(&text[start..]))
        {
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
