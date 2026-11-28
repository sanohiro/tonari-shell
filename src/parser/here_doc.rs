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

#[derive(Clone)]
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
        return Err(Error::incomplete_message(
            "continued here-document operator",
        ));
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
            && (separator(c) || !started && c == '#' && super::READ_OPTIONS.get().comments)
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
                let next = chars.next().ok_or(Error::incomplete_message(
                    "unfinished here-document delimiter",
                ))?;
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
                    let ch = chars.next().ok_or(Error::incomplete_message(
                        "unterminated backquote in here-document delimiter",
                    ))?;
                    text.push(ch);
                    if ch == '`' {
                        break;
                    }
                    if ch == '\\'
                        && let Some(escaped) = chars.next()
                    {
                        text.push(escaped);
                    }
                }
            }
            _ => text.push(c),
        }
        started = true;
    }
    if quote.is_some() || depth != 0 {
        return Err(Error::incomplete_message(
            "unterminated here-document delimiter",
        ));
    }
    if !started {
        return Err(Error::Invalid("expected a here-document delimiter"));
    }
    Ok((text, quoted))
}

// An early substitution closer can place a here-document body after its
// surrounding word. Keep physical ranges only until the outer lexer returns.
thread_local! {
    static SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static RANGES: std::cell::RefCell<Vec<(usize,usize)>> = const { std::cell::RefCell::new(Vec::new()) };
    static ANCESTORS: std::cell::RefCell<Vec<Pending>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(super) struct Scan;
impl Scan {
    pub(super) fn enter() -> Self {
        SCANS.set(SCANS.get() + 1);
        Self
    }
}
impl Drop for Scan {
    fn drop(&mut self) {
        let depth = SCANS.get() - 1;
        SCANS.set(depth);
        if depth == 0 {
            RANGES.with_borrow_mut(Vec::clear);
        }
    }
}

pub(super) struct Ancestors(usize);
pub(super) fn remember(earlier: &[Pending]) -> Ancestors {
    ANCESTORS.with_borrow_mut(|v| {
        let start = v.len();
        v.extend_from_slice(earlier);
        Ancestors(start)
    })
}
impl Drop for Ancestors {
    fn drop(&mut self) {
        ANCESTORS.with_borrow_mut(|v| v.truncate(self.0));
    }
}

pub(super) fn skip(chars: &mut Chars<'_>) {
    RANGES.with_borrow(|ranges| {
        while let Some(&(_, len)) = ranges
            .iter()
            .find(|&&(addr, n)| addr == chars.as_str().as_ptr().addr() && n <= chars.as_str().len())
        {
            *chars = chars.as_str()[len..].chars();
        }
    });
}

pub(super) fn after_closer(
    source: &str,
    after: &str,
    tokens: &mut [Token],
    pending: &mut Vec<Pending>,
) -> Result<(), Error> {
    if pending.is_empty() {
        return Ok(());
    }
    let tail = after.split_once('\n').map_or("", |(_, tail)| tail);
    let mut cursor = tail.chars();
    ANCESTORS.with_borrow(|earlier| -> Result<(), Error> {
        for doc in earlier {
            skip(&mut cursor);
            body(&mut cursor, doc, false, false)?;
        }
        Ok(())
    })?;
    skip(&mut cursor);
    let start = cursor.as_str();
    collect(source.len(), &mut cursor, tokens, pending, false)?;
    let consumed = start.len() - cursor.as_str().len();
    if consumed > 0 {
        RANGES.with_borrow_mut(|ranges| ranges.push((start.as_ptr().addr(), consumed)));
    }
    Ok(())
}

fn body(
    chars: &mut Chars<'_>,
    doc: &Pending,
    final_input: bool,
    capture: bool,
) -> Result<String, Error> {
    let mut body = String::new();
    let mut continued = String::new();
    let mut was_continued = false;
    loop {
        let remaining = chars.as_str();
        if remaining.is_empty() {
            if !final_input {
                return Err(Error::HereDoc(HereDocHint {
                    delimiter: doc.delimiter.clone(),
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
        let continuation = !doc.quoted && remaining.len() > physical.len() && continues(physical);
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
        if capture {
            body.push_str(line);
            body.push('\n');
        }
        continued.clear();
    }
    Ok(body)
}

pub(super) fn collect(
    source_len: usize,
    chars: &mut Chars<'_>,
    tokens: &mut [Token],
    pending: &mut Vec<Pending>,
    final_input: bool,
) -> Result<(), Error> {
    for doc in pending.drain(..) {
        skip(chars);
        let start = source_len - chars.as_str().len();
        let text = body(chars, &doc, final_input, true)?;
        tokens[doc.index] = Token::HereBody(text, (start, source_len - chars.as_str().len()));
    }
    skip(chars);
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
