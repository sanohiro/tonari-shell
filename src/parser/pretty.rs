use super::{Error, Token, lex, lexer, with_read_options};
use std::borrow::Cow;

// Keep the shell's original words, operators and top-level read boundaries.
// In particular, adding a top-level newline can change when aliases take effect.
pub(crate) fn format<'a>(source: &'a str, session: &crate::Session) -> Result<Cow<'a, str>, Error> {
    with_read_options(session, 1, || {
        if source.contains('\0') {
            return Err(Error::NulByte);
        }
        let (tokens, spans, _) = lex(source, false, true)?;
        let mut reader = super::Reader::new(tokens, spans, source);
        reader.final_input = true;
        if source.contains("<<") || source.contains('#') || source.contains("\\\n") {
            reader.list(&[])?;
            return Ok(Cow::Borrowed(source));
        }
        let tokens = reader.tokens.make_contiguous();
        let spans = &reader.spans;
        if tokens.iter().any(Token::preserve_layout) {
            reader.list(&[])?;
            return Ok(Cow::Borrowed(source));
        }
        let mut result = String::new();
        let mut copied = 0;
        let mut depth: usize = 0;
        let mut after_body_start = false;
        let mut after_separator = false;
        for (i, (token, &(start, end))) in tokens.iter().zip(spans).enumerate() {
            if matches!(token, Token::Newline) {
                after_separator = true;
                continue;
            }
            let word = token.keyword().filter(|word| {
                lexer::command_position(&tokens[..i])
                    || *word == "do" && i > 0 && matches!(tokens[i - 1], Token::Arithmetic(_))
            });
            let closing = matches!(word, Some("fi" | "done" | "elif" | "else"));
            if closing {
                depth = depth.saturating_sub(1);
            }
            let attach = matches!(word, Some("then" | "do"));
            if (after_body_start || after_separator && (depth > 0 || closing)) && !attach {
                let gap = source[..start].trim_end_matches([' ', '\t', '\n']).len();
                result.push_str(&source[copied..gap]);
                result.push('\n');
                for _ in 0..depth {
                    result.push_str("    ");
                }
                copied = start;
            } else if after_separator && attach && i > 0 {
                let last = (0..i)
                    .rev()
                    .find(|&index| !matches!(tokens[index], Token::Newline));
                if let Some(last) = last {
                    result.push_str(&source[copied..spans[last].1]);
                    if matches!(tokens[last], Token::End) {
                        result.push(' ');
                    } else {
                        result.push_str("; ");
                    }
                    copied = start;
                }
            }
            after_body_start = matches!(word, Some("then" | "do" | "else"));
            if after_body_start {
                depth += 1;
            }
            after_separator = matches!(token, Token::End);
            debug_assert!(end <= source.len());
        }
        reader.list(&[])?;
        if copied == 0 {
            Ok(Cow::Borrowed(source))
        } else {
            result.push_str(&source[copied..]);
            Ok(Cow::Owned(result))
        }
    })
}
