// A one-command brace function is common in rc files. Build the ordinary AST
// directly when its word boundaries are unambiguous. The full grammar handles
// aliases, lists, redirections, assignments and compound or concatenated words.
use super::{
    Command, Condition, Location, SimpleCommand, Statement, Word, aliases, lexer, reserved,
};
use crate::word::{WordBuilder, is_identifier};
use std::rc::Rc;

fn name(header: &str) -> Option<&str> {
    let header = header.trim_matches([' ', '\t']);
    let name = match header
        .strip_prefix("function")
        .filter(|rest| rest.starts_with([' ', '\t']))
    {
        Some(rest) => rest.strip_suffix("()").unwrap_or(rest),
        None => header.strip_suffix("()")?,
    }
    .trim_matches([' ', '\t']);
    (is_identifier(name) && !reserved(name)).then_some(name)
}

pub(crate) fn opens_function(line: &str) -> bool {
    line.strip_suffix('\n')
        .unwrap_or(line)
        .trim_end_matches([' ', '\t'])
        .strip_suffix('{')
        .and_then(name)
        .is_some()
}

pub(super) fn parse(source: &str) -> Option<Statement> {
    let line = source
        .strip_suffix('\n')
        .unwrap_or(source)
        .trim_end_matches([' ', '\t']);
    let (header, body) = line.strip_suffix('}')?.split_once('{')?;
    let name = name(header)?;
    if !body.starts_with([' ', '\t', '\n']) || aliases::current().0.is_some() {
        return None;
    }
    // The command must end before the closing brace. A newline or a semicolon
    // terminates it; a space alone is part of the same simple command.
    let tail = body.trim_end_matches([' ', '\t', '\n']);
    let command = tail
        .strip_suffix(';')
        .or_else(|| body[tail.len()..].contains('\n').then_some(tail))?;
    let text = command.trim_matches([' ', '\t', '\n']);
    if text.is_empty() || text.contains('\n') {
        return None;
    }
    let mut rest = text;
    let mut words = Vec::with_capacity(2);
    while !rest.is_empty() {
        let first = rest.as_bytes()[0];
        let word = if matches!(first, b'\'' | b'"') && !words.is_empty() {
            let mut chars = rest[1..].chars();
            let mut word = WordBuilder::default();
            lexer::quoted(&mut chars, first as char, &mut word).ok()?;
            rest = chars.as_str();
            word.finish()
        } else {
            let end = rest
                .bytes()
                .take_while(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'/' | b'.' | b':')
                })
                .count();
            let (word, tail) = rest.split_at(end);
            if word.is_empty() || words.is_empty() && reserved(word) {
                return None;
            }
            rest = tail;
            Word::Literal(word.to_owned())
        };
        if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
            return None;
        }
        words.push(word);
        rest = rest.trim_start_matches([' ', '\t']);
    }
    let source_text: Rc<str> = source.into();
    let location = |start, end, line| Location {
        source: source_text.clone(),
        start,
        end,
        line,
    };
    let open = header.len();
    let layout = command.len() - command.trim_start_matches([' ', '\t', '\n']).len();
    let start = open + 1 + layout;
    let line_number = super::FIRST_LINE.get();
    let command_line =
        line_number + memchr::memchr_iter(b'\n', &command.as_bytes()[..layout]).count();
    let command = Statement {
        location: location(start, start + text.len(), command_line),
        condition: Condition::Always,
        negated: false,
        command: Command::Pipeline(vec![SimpleCommand {
            words,
            ..Default::default()
        }]),
    };
    Some(Statement {
        location: location(
            header.len() - header.trim_start_matches([' ', '\t']).len(),
            line.len(),
            line_number,
        ),
        condition: Condition::Always,
        negated: false,
        command: Command::Function {
            name: name.to_owned(),
            definition: Rc::new(super::FunctionDefinition {
                body: vec![command],
                location: location(open, line.len(), line_number),
            }),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_functions_have_the_full_parsers_words_and_source_locations() {
        for header in ["f()", "  f  ()", "\tfunction f", "function f()"] {
            for body in [
                " :; ",
                "\n\tprintf '%s;' \"$@\"\n",
                "\n\n echo \"雪 $1\"; \n",
                " echo \"$(printf x)\" '' ;",
            ] {
                let source = format!("{header} {{ {body}}}\t\n");
                let fast = parse(&source).unwrap();
                let (tokens, spans, _) = lexer::lex(&source, false, true).unwrap();
                let normal = super::super::Reader::new(tokens, spans, &source)
                    .list(&[])
                    .unwrap();
                assert_eq!(
                    format!("{fast:?}"),
                    format!("{:?}", normal[0]),
                    "{source:?}"
                );
            }
        }
    }

    #[test]
    fn ambiguous_or_incomplete_definitions_use_the_full_grammar() {
        for source in [
            "f(){ echo hi }",
            "f(){ echo a; echo b; }",
            "f(){ x=1 cmd; }",
            "f(){ echo 'x'y; }",
            "f(){ echo x >out; }",
            "f(){ echo *; }",
            "f(){ echo x; } >out",
            "f(){ if :; then :; fi; }",
            "f(){ echo 'bad; }",
            "f(){ echo x # a; }",
            "f(){ echo \\}; }",
            "if(){ echo x; }",
        ] {
            assert!(parse(source).is_none(), "{source}");
        }
    }
}
