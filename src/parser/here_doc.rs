use super::Incomplete;
use super::lexer::{LexWord, Token, backquote, dollar, separator};
use super::{Error, Target};
use crate::word::{Word, WordBuilder};
use std::str::Chars;

#[derive(Debug)]
pub struct HereDocHint {
    delimiter: String,
    quoted: bool,
    strip_tabs: bool,
    continued: bool,
}

fn continues(line: &str) -> bool {
    line.as_bytes()
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

impl HereDocHint {
    /// A continued logical line may assemble its delimiter from multiple reads.
    /// A suffix hit triggers the authoritative parser comparison.
    pub fn may_finish(&mut self, physical: &str) -> bool {
        let line = physical.strip_suffix('\n').unwrap_or(physical);
        let previous = self.continued;
        self.continued = !self.quoted && physical.ends_with('\n') && continues(line);
        if self.continued {
            return false;
        }
        let line = if self.strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line
        };
        if previous {
            self.delimiter.ends_with(line)
        } else {
            self.delimiter == line
        }
    }
}

pub(super) struct Pending {
    index: usize,
    delimiter: String,
    quoted: bool,
    strip_tabs: bool,
}

pub(super) fn redirect(
    chars: &mut Chars<'_>,
    fd: i32,
    tokens: &mut Vec<Token>,
    pending: &mut Vec<Pending>,
) -> Result<(), Error> {
    chars.next();
    *chars = super::lexer::joined(chars.as_str()).chars();
    if chars.as_str().starts_with('<') {
        chars.next();
        tokens.push(Token::Redirect(fd, Target::HereString));
        return Ok(());
    }
    let strip_tabs = chars.as_str().starts_with('-');
    if strip_tabs {
        chars.next();
    }
    loop {
        *chars = super::lexer::joined(chars.as_str()).chars();
        if !matches!(chars.clone().next(), Some(' ' | '\t')) {
            break;
        }
        chars.next();
    }
    if chars.as_str().starts_with("\\\n") && chars.as_str().trim_start_matches("\\\n").is_empty() {
        return Err(Error::Incomplete(Incomplete::Message(
            "continued here-document operator",
        )));
    }
    let (delimiter, quoted) = delimiter(chars)?;
    tokens.push(Token::Redirect(fd, Target::HereDoc { expand: !quoted }));
    pending.push(Pending {
        index: tokens.len(),
        delimiter,
        quoted,
        strip_tabs,
    });
    tokens.push(body_token(String::new()));
    Ok(())
}

fn body_token(body: String) -> Token {
    Token::Word(LexWord {
        word: Word::Literal(body),
        protected: true,
        assignment: None,
    })
}

// A here-document delimiter undergoes quote removal, but no expansion. Nested
// command syntax can contain separators that still belong to the same word.
fn delimiter(chars: &mut Chars<'_>) -> Result<(String, bool), Error> {
    let mut text = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut quote = None;
    let mut depth = 0usize;
    while let Some(c) = chars.clone().next() {
        if quote.is_none()
            && depth == 0
            && (separator(c) || !started && c == '#' && super::COMMENTS.get())
        {
            break;
        }
        chars.next();
        match c {
            '\'' | '"' if quote.is_none() => {
                quote = Some(c);
                quoted = true;
            }
            _ if quote == Some(c) => quote = None,
            '\\' if quote != Some('\'') => {
                let next = chars.next().ok_or(Error::Incomplete(Incomplete::Message(
                    "unfinished here-document delimiter",
                )))?;
                if next == '\n' {
                    continue;
                }
                if quote == Some('"') && !matches!(next, '$' | '`' | '"' | '\\') {
                    text.push('\\');
                }
                text.push(next);
                quoted = true;
            }
            '$' if quote.is_none() && chars.as_str().starts_with('\'') => {
                chars.next();
                text.push_str(&super::ansi::quoted(chars)?);
                quoted = true;
            }
            '$' if quote.is_none() && chars.as_str().starts_with('(') => {
                chars.next();
                text.push_str("$(");
                depth += 1;
            }
            '(' if quote.is_none() && depth > 0 => {
                depth += 1;
                text.push(c);
            }
            ')' if quote.is_none() && depth > 0 => {
                depth -= 1;
                text.push(c);
            }
            '`' if quote.is_none() => {
                text.push(c);
                loop {
                    let ch = chars.next().ok_or(Error::Incomplete(Incomplete::Message(
                        "unterminated backquote in here-document delimiter",
                    )))?;
                    text.push(ch);
                    if ch == '`' {
                        break;
                    }
                    if ch == '\\' && let Some(escaped) = chars.next() {
                        text.push(escaped);
                    }
                }
            }
            _ => text.push(c),
        }
        started = true;
    }
    if quote.is_some() || depth != 0 {
        return Err(Error::Incomplete(Incomplete::Message(
            "unterminated here-document delimiter",
        )));
    }
    if !started {
        return Err(Error::Invalid("expected a here-document delimiter"));
    }
    Ok((text, quoted))
}

pub(super) fn collect(
    source_len: usize,
    chars: &mut Chars<'_>,
    tokens: &mut [Token],
    pending: &mut Vec<Pending>,
    final_input: bool,
) -> Result<(), Error> {
    for doc in pending.drain(..) {
        let start = source_len - chars.as_str().len();
        let mut body = String::new();
        let mut continued = String::new();
        let mut was_continued = false;
        loop {
            let remaining = chars.as_str();
            if remaining.is_empty() {
                if !final_input {
                    return Err(Error::HereDoc(HereDocHint {
                        delimiter: doc.delimiter,
                        quoted: doc.quoted,
                        strip_tabs: doc.strip_tabs,
                        continued: was_continued,
                    }));
                }
                diagnostic!(
                    "warning: here-document reached end of input (wanted '{}')",
                    doc.delimiter
                );
                if !continued.is_empty() {
                    body.push_str(&continued);
                    body.push('\n');
                }
                break;
            }
            let (physical, rest) = remaining.split_once('\n').unwrap_or((remaining, ""));
            *chars = rest.chars();
            let continuation =
                !doc.quoted && remaining.len() > physical.len() && continues(physical);
            was_continued = continuation;
            if continuation {
                continued.push_str(&physical[..physical.len() - 1]);
                continue;
            }
            let line = if continued.is_empty() {
                physical
            } else {
                continued.push_str(physical);
                &continued
            };
            let line = if doc.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if line == doc.delimiter {
                break;
            }
            body.push_str(line);
            body.push('\n');
            continued.clear();
        }
        tokens[doc.index] = Token::HereBody(body, (start, source_len - chars.as_str().len()));
    }
    Ok(())
}

pub(crate) fn expansion_string(source: &str) -> Result<Word, Error> {
    let mut result = WordBuilder::default();
    let mut chars = source.chars();
    loop {
        let rest = chars.as_str();
        let Some(idx) = rest.find(['$', '`', '\\']) else {
            result.literal(rest);
            break;
        };
        result.literal(&rest[..idx]);
        chars = rest[idx..].chars();
        match chars.next().unwrap() {
            '$' => dollar(&mut chars, &mut result, true)?,
            '`' => backquote(&mut chars, &mut result, true)?,
            '\\' => match chars.clone().next() {
                Some('$' | '`' | '\\') => result.push(chars.next().unwrap()),
                Some('\n') => {
                    chars.next();
                }
                _ => result.push('\\'),
            },
            _ => unreachable!(),
        }
    }
    Ok(result.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Command, parse, parse_final};

    fn redirects(source: &str) -> Vec<(Target, Word)> {
        let Command::Pipeline(mut pipeline) = parse(source).unwrap().remove(0).command else {
            panic!()
        };
        pipeline
            .remove(0)
            .redirects
            .into_iter()
            .map(|r| (r.target, r.word))
            .collect()
    }

    #[test]
    fn consumes_multiple_delimiters_in_source_order() {
        let docs = redirects("cat <<-A 0<<'B C' ; echo next\n\tfirst\n\tA\n\tsecond $x\nB C");
        assert_eq!(docs[0].0, Target::HereDoc { expand: true });
        assert_eq!(docs[0].1.as_literal(), Some("first\n"));
        assert_eq!(docs[1].0, Target::HereDoc { expand: false });
        assert_eq!(docs[1].1.as_literal(), Some("\tsecond $x\n"));
    }

    #[test]
    fn joins_unquoted_continuations_before_tab_stripping_and_marker_test() {
        let docs = redirects("cat <<-END\n\tfirst\\\n\tsecond\n\tE\\\nND\necho next");
        assert_eq!(docs[0].1.as_literal(), Some("first\tsecond\n"));
        let docs = redirects("cat <<'END'\nfirst\\\nEND\n");
        assert_eq!(docs[0].1.as_literal(), Some("first\\\n"));
    }

    #[test]
    fn here_string_and_here_document_eof() {
        let docs = redirects("cat <<< \"hi $x\"");
        assert_eq!(docs[0].0, Target::HereString);
        assert_eq!(
            docs[0].1.clone().scalar(&|_| Some("there".into()), &[]),
            "hi there"
        );
        assert!(matches!(parse("cat <<END\nsome\n"), Err(Error::HereDoc(_))));
        let Command::Pipeline(mut pipeline) =
            parse_final("cat <<END\nsome").unwrap().remove(0).command
        else {
            panic!()
        };
        assert_eq!(
            pipeline.remove(0).redirects[0].word.as_literal(),
            Some("some\n")
        );
    }

    #[test]
    fn delays_here_doc_substitutions_until_execution() {
        parse("false && cat <<EOF\n$(\nEOF\n").unwrap();
        let text = r#"'literal quotes' "\$x" \\$x ${y:-"test"}"#;
        let words = expansion_string(text).unwrap();
        let value = words.scalar(&|s| (s == "x").then(|| "word".into()), &[]);
        assert_eq!(value, r#"'literal quotes' "$x" \word test"#);
    }
}
