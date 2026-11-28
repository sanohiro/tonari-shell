use super::*;

fn with_tokens<T>(source: &str, read: impl FnOnce(&[Token], &[(usize, usize)]) -> T) -> Option<T> {
    let (mut tokens, mut spans, _) = lex(source, false, false).ok()?;
    let result = read(&tokens, &spans);
    if tokens.capacity().max(spans.capacity()) <= MAX_CACHED_TOKENS {
        tokens.clear();
        spans.clear();
        TOKEN_BUFFERS.set((tokens, spans));
    }
    Some(result)
}

// Fold only token-separating newlines. Quoted newlines, comment endings and
// here-document boundaries must survive recall and execution.
pub(crate) fn history_command(text: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if text.contains(['#', '\x60']) || text.contains("<<") {
        return Cow::Borrowed(text);
    }
    with_tokens(text, |tokens, spans| {
        if tokens
            .iter()
            .any(|t| t.preserve_layout() || t.keyword() == Some("function"))
        {
            return Cow::Borrowed(text);
        }
        let mut result = String::new();
        let mut start = 0;
        for (i, &(a, b)) in spans.iter().enumerate() {
            if !matches!(tokens[i], Token::Newline) || i == 0 {
                continue;
            }
            let previous = &tokens[i - 1];
            if matches!(previous, Token::RightParen)
                || tokens.get(i + 1).and_then(Token::keyword) == Some("in")
            {
                continue;
            }
            result.push_str(&text[start..a]);
            if !lexer::command_position(&tokens[..i]) {
                result.push(';');
            }
            result.push(' ');
            start = b;
        }
        if start == 0 {
            Cow::Borrowed(text)
        } else {
            result.push_str(&text[start..]);
            Cow::Owned(result)
        }
    })
    .unwrap_or(Cow::Borrowed(text))
}

pub(crate) fn history_word_spans(s: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if !escaped && quote.is_none() && c.is_whitespace() {
            if let Some(a) = start.take() {
                words.push((a, i));
            }
            continue;
        }
        start.get_or_insert(i);
        if escaped {
            escaped = false;
        } else if c == '\\' && quote != Some('\'') {
            escaped = true;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"') {
            quote = Some(c);
        }
    }
    if let Some(a) = start {
        words.push((a, s.len()));
    }
    words
}

// Readline groups an adjacent decimal descriptor and one optional move/close
// marker with <& or >&. Any remaining text starts a separate history word.
fn history_fd_end(
    line: &str,
    tokens: &[Token],
    spans: &[(usize, usize)],
    index: usize,
) -> Option<usize> {
    let previous = index.checked_sub(1)?;
    if !matches!(
        tokens[previous],
        Token::Redirect(_, Target::Duplicate | Target::OutputWord)
    ) {
        return None;
    }
    let (start, end) = spans[index];
    if spans[previous].1 != start {
        return None;
    }
    let bytes = &line.as_bytes()[start..end];
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    let consumed = digits + usize::from(bytes.get(digits) == Some(&b'-'));
    (consumed != 0).then_some(start + consumed)
}

pub(crate) fn history_designator_spans(line: &str, extglob: bool) -> Vec<(usize, usize)> {
    if !line.bytes().any(|b| {
        matches!(
            b,
            b'\''
                | b'"'
                | b'\\'
                | b'$'
                | b'`'
                | b';'
                | b'|'
                | b'&'
                | b'<'
                | b'>'
                | b'('
                | b')'
                | b'#'
        )
    }) {
        return history_word_spans(line);
    }
    let _extglob = crate::ScopedCell::enter(
        &READ_OPTIONS,
        ReadOptions {
            extglob,
            ..READ_OPTIONS.get()
        },
    );
    with_tokens(line, |tokens, spans| {
        let mut result: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
        for (i, (token, &(mut start, end))) in tokens.iter().zip(spans).enumerate() {
            if let Some(split) = history_fd_end(line, tokens, spans, i)
                && let Some(previous) = result.last_mut()
            {
                previous.1 = split;
                start = split;
            }
            if start < end && !matches!(token, Token::Newline) {
                result.push((start, end));
            }
        }
        result
    })
    .unwrap_or_else(|| history_word_spans(line))
}

/// Keep the original shell spelling, including quotes and substitutions.
pub(crate) fn last_history_argument(line: &str) -> Option<&str> {
    let blank = [' ', '\t', '\n'];
    let trimmed = line.trim_end_matches(blank);
    if trimmed.is_empty() {
        return None;
    }

    // A bare prefix cannot leave the last argument inside an open quote or expansion.
    let first_special = trimmed.bytes().position(|b| {
        !(lexer::plain_byte(b)
            || b >= 128
            || matches!(b, b' ' | b'\t' | b'\n' | b'=' | b',' | b'@' | b'%'))
    });
    let prefix = match first_special {
        None => return trimmed.rsplit(blank).next(),
        Some(i) => &trimmed[..i],
    };
    let tail = &trimmed.as_bytes()[prefix.len()..];
    if tail.len() >= 2
        && tail[0] == b'\''
        && memchr::memchr(b'\'', &tail[1..]) == Some(tail.len() - 2)
    {
        let start = prefix.rfind(blank).map_or(0, |i| i + 1);
        return Some(&trimmed[start..]);
    }
    // Pass the untrimmed line to the lexer: an escaped trailing blank belongs to the word.
    with_tokens(line, |tokens, spans| {
        let index = tokens
            .iter()
            .zip(spans)
            .rposition(|(t, &(a, b))| !matches!(t, Token::Newline) || b - a > 1)?;
        if matches!(tokens[index], Token::Newline) {
            // A newline spanning a heredoc includes its closing delimiter.
            let (a, b) = spans[index];
            let closing = line[a..b].trim_end_matches('\n').rsplit('\n').next()?;
            return last_history_argument(closing);
        }
        let (start, end) = spans[index];
        let start = match history_fd_end(line, tokens, spans, index) {
            Some(split) if split == end => spans[index - 1].0,
            Some(split) => split,
            None => start,
        };
        Some(&line[start..end])
    })
    .unwrap_or_else(|| history_word_spans(line).last().map(|&(a, b)| &line[a..b]))
}

/// Byte end of the next shell token in the displayed command, including any
/// whitespace before it. Lexing identifies quotes and substitutions without running them.
pub(crate) fn suggestion_end(line: &str, cursor: usize) -> usize {
    // A plain prefix cannot leave us inside quotes or a substitution. Stop
    // at the next blank without lexing later arguments of a long suggestion.
    let bare = |b: u8| b.is_ascii_alphanumeric() || b >= 128 || b"-_./:=,@%+".contains(&b);
    let tail = line[cursor..].trim_start_matches([' ', '\t']);
    let word = tail.bytes().take_while(|&b| bare(b)).count();
    if word > 0
        && tail
            .as_bytes()
            .get(word)
            .is_none_or(|b| matches!(b, b' ' | b'\t' | b'\n'))
        && line[..cursor]
            .bytes()
            .all(|b| bare(b) || matches!(b, b' ' | b'\t'))
    {
        return line.len() - tail.len() + word;
    }
    let next_end = |source| {
        with_tokens(source, |_, spans| {
            spans
                .iter()
                .find(|(_, end)| *end > cursor)
                .map(|&(_, end)| end)
        })
        .flatten()
    };
    next_end(line)
        .or_else(|| {
            // History can contain syntax errors beyond this argument. Confirm a
            // shorter prefix with the lexer before copying any of it into the draft.
            let (_, end) = history_word_spans(line)
                .into_iter()
                .find(|&(_, end)| end > cursor)?;
            next_end(&line[..end])
        })
        .unwrap_or(cursor)
}
