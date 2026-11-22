use std::str::Chars;

#[derive(Debug, PartialEq)]
pub enum Target {
    Read,
    Write,
    Append,
    Duplicate,
}

#[derive(Debug)]
pub struct Redirect {
    pub fd: i32,
    pub target: Target,
    pub word: String,
}

#[derive(Debug, Default)]
pub struct SimpleCommand {
    pub words: Vec<String>,
    pub redirects: Vec<Redirect>,
}

pub type Pipeline = Vec<SimpleCommand>;

#[derive(Debug)]
pub enum Error {
    Incomplete(&'static str),
    Invalid(&'static str),
}

impl Error {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Incomplete(s) | Self::Invalid(s) => s,
        }
    }
}

enum Token {
    Word { text: String, quoted: bool },
    Redirect(i32, Target),
    Pipe,
    End,
    Newline,
}

fn descriptor(s: &str) -> Result<i32, Error> {
    match s {
        "0" => Ok(0),
        "1" => Ok(1),
        "2" => Ok(2),
        _ => Err(Error::Invalid(
            "only file descriptors 0, 1 and 2 are supported",
        )),
    }
}

fn redirect(chars: &mut Chars<'_>, op: char, fd: i32) -> Result<Token, Error> {
    let target = match (op, chars.clone().next()) {
        ('<', Some('<' | '>')) | ('>', Some('|')) => {
            return Err(Error::Invalid("unsupported redirection"));
        }
        (_, Some('&')) => {
            chars.next();
            Target::Duplicate
        }
        ('>', Some('>')) => {
            chars.next();
            Target::Append
        }
        ('>', _) => Target::Write,
        _ => Target::Read,
    };
    Ok(Token::Redirect(fd, target))
}

fn quoted(chars: &mut Chars<'_>, quote: char, word: &mut String) -> Result<(), Error> {
    if quote == '\'' {
        let (literal, rest) = chars
            .as_str()
            .split_once(quote)
            .ok_or(Error::Incomplete("unterminated quote"))?;
        word.push_str(literal);
        *chars = rest.chars();
        return Ok(());
    }
    while let Some(c) = chars.next() {
        if c == quote {
            return Ok(());
        }
        match c {
            '\\' if matches!(chars.clone().next(), Some('"' | '$' | '`' | '\\' | '\n')) => {
                let escaped = chars.next().unwrap();
                if escaped != '\n' {
                    word.push(escaped);
                }
                continue;
            }
            '$' | '`' => return Err(Error::Invalid("shell expansion is not implemented yet")),
            _ => {}
        }
        word.push(c);
    }
    Err(Error::Incomplete("unterminated quote"))
}

fn lex(source: &str) -> Result<Vec<Token>, Error> {
    if source.contains('\0') {
        return Err(Error::Invalid("NUL in command text"));
    }
    let mut chars = source.chars();
    let mut tokens = Vec::new();
    while let Some(c) = chars.clone().next() {
        if matches!(c, ' ' | '\t') {
            chars.next();
            continue;
        }
        if c == '#' {
            let rest = chars.as_str();
            chars = rest[rest.find('\n').unwrap_or(rest.len())..].chars();
            continue;
        }
        if matches!(c, '\n' | ';' | '|' | '<' | '>') {
            chars.next();
            tokens.push(match c {
                '\n' => Token::Newline,
                ';' => Token::End,
                '|' => Token::Pipe,
                op => redirect(&mut chars, op, if op == '<' { 0 } else { 1 })?,
            });
            continue;
        }
        let mut word = String::new();
        let mut protected = false;
        let mut started = false;
        while let Some(c) = chars.clone().next() {
            if matches!(c, ' ' | '\t' | '\n' | ';' | '|' | '<' | '>') {
                break;
            }
            chars.next();
            match c {
                '\'' | '"' => {
                    protected = true;
                    quoted(&mut chars, c, &mut word)?;
                }
                '\\' => {
                    let escaped = chars
                        .next()
                        .ok_or(Error::Incomplete("trailing backslash"))?;
                    if escaped == '\n' {
                        if chars.as_str().is_empty() {
                            return Err(Error::Incomplete("line continuation"));
                        }
                        continue;
                    }
                    protected = true;
                    word.push(escaped);
                }
                '&' | '(' | ')' => return Err(Error::Invalid("unsupported shell operator")),
                '$' | '`' | '*' | '?' | '[' | '{' => {
                    return Err(Error::Invalid("shell expansion is not implemented yet"));
                }
                '~' if !started => {
                    return Err(Error::Invalid("tilde expansion is not implemented yet"));
                }
                _ => word.push(c),
            }
            started = true;
        }
        if !started {
            continue;
        }
        if !protected
            && word.bytes().all(|b| b.is_ascii_digit())
            && matches!(chars.clone().next(), Some('<' | '>'))
        {
            let op = chars.next().unwrap();
            tokens.push(redirect(&mut chars, op, descriptor(&word)?)?);
        } else {
            tokens.push(Token::Word {
                text: word,
                quoted: protected,
            });
        }
    }
    Ok(tokens)
}

pub fn parse(source: &str) -> Result<Vec<Pipeline>, Error> {
    let mut tokens = lex(source)?.into_iter();
    let mut program = Vec::new();
    let mut pipeline = Vec::new();
    let mut cmd = SimpleCommand::default();
    while let Some(token) = tokens.next() {
        match token {
            Token::Word { text, quoted } => {
                if cmd.words.is_empty()
                    && !quoted
                    && matches!(
                        text.as_str(),
                        "if" | "then"
                            | "else"
                            | "elif"
                            | "fi"
                            | "case"
                            | "esac"
                            | "while"
                            | "until"
                            | "for"
                            | "do"
                            | "done"
                            | "in"
                            | "select"
                            | "function"
                            | "coproc"
                            | "time"
                            | "!"
                            | "}"
                    )
                {
                    return Err(Error::Invalid("compound commands are not implemented yet"));
                }
                if cmd.words.is_empty()
                    && text.split_once('=').is_some_and(|(name, _)| {
                        !name.is_empty()
                            && !name.as_bytes()[0].is_ascii_digit()
                            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    })
                {
                    return Err(Error::Invalid("shell assignment is not implemented yet"));
                }
                cmd.words.push(text);
            }
            Token::Redirect(fd, target) => {
                let Some(Token::Word { text: word, .. }) = tokens.next() else {
                    return Err(Error::Invalid("expected a redirection target"));
                };
                if target == Target::Duplicate && word != "-" {
                    descriptor(&word)?;
                }
                cmd.redirects.push(Redirect { fd, target, word });
            }
            Token::Newline if cmd.words.is_empty() && cmd.redirects.is_empty() => continue,
            Token::Pipe | Token::End | Token::Newline => {
                if cmd.words.is_empty() && cmd.redirects.is_empty() {
                    return Err(Error::Invalid("expected a command"));
                }
                pipeline.push(std::mem::take(&mut cmd));
                if !matches!(token, Token::Pipe) {
                    program.push(std::mem::take(&mut pipeline));
                }
            }
        }
    }
    if !cmd.words.is_empty() || !cmd.redirects.is_empty() {
        pipeline.push(cmd);
        program.push(pipeline);
    } else if !pipeline.is_empty() {
        return Err(Error::Incomplete("expected a command after pipe"));
    }
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_quotes_empty_arguments_and_utf8_through_lexing() {
        let parsed = parse(r#"printf '' "こ"'ん'\ にちは "#).unwrap();
        assert_eq!(parsed[0][0].words, ["printf", "", "こん にちは"]);
    }

    #[test]
    fn single_quoted_spans_keep_metacharacters_and_newlines_literal() {
        let source = "run pre'世界\n# $HOME \\` *'post # ignored\nnext";
        let parsed = parse(source).unwrap();
        assert_eq!(parsed[0][0].words, ["run", "pre世界\n# $HOME \\` *post"]);
        assert_eq!(parsed[1][0].words, ["next"]);
    }

    #[test]
    fn recognizes_only_unquoted_adjacent_io_numbers() {
        let parsed = parse(r#"cmd 2>log 2 >out '2'>last 1>&2"#).unwrap();
        let cmd = &parsed[0][0];
        assert_eq!(cmd.words, ["cmd", "2", "2"]);
        let fds: Vec<_> = cmd.redirects.iter().map(|r| r.fd).collect();
        assert_eq!(fds, [2, 1, 1, 1]);
        assert_eq!(cmd.redirects[3].target, Target::Duplicate);
    }

    #[test]
    fn distinguishes_continuation_from_invalid_input() {
        for source in ["cmd |\n", "cmd '", "cmd \\\n"] {
            assert!(matches!(parse(source), Err(Error::Incomplete(_))));
        }
        for source in ["| cmd", "cmd || cmd", "cmd > ", "cmd 9>&2"] {
            assert!(matches!(parse(source), Err(Error::Invalid(_))));
        }
        let parsed = parse("cmd |\n# comment\ncat; next\n").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].len(), 2);
    }
}
