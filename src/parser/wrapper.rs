// A one-command brace function is common in rc files. Build the ordinary AST
// directly when its word boundaries are unambiguous. The full grammar handles
// aliases, lists, redirections, assignments and compound or concatenated words.
use super::{
    Command, Condition, Location, SimpleCommand, Statement, Word, aliases, lexer, reserved,
};
use crate::word::{WordBuilder, is_identifier};
use std::rc::Rc;

pub(super) fn parse(source: &str) -> Option<Statement> {
    let line = source
        .strip_suffix('\n')
        .unwrap_or(source)
        .trim_end_matches([' ', '\t']);
    let (header, body) = line.strip_suffix('}')?.split_once('{')?;
    let name = header
        .trim_end_matches([' ', '\t'])
        .strip_suffix("()")?
        .trim_matches([' ', '\t']);
    if !is_identifier(name)
        || reserved(name)
        || !body.starts_with([' ', '\t'])
        || aliases::current().0.is_some()
    {
        return None;
    }
    let body = body.trim_end_matches([' ', '\t']).strip_suffix(';')?;
    let text = body.trim_matches([' ', '\t']);
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
    let location = |start, end| Location {
        source: source_text.clone(),
        start,
        end,
        line: super::FIRST_LINE.get(),
    };
    let open = header.len();
    let start = open + 1 + body.len() - body.trim_start_matches([' ', '\t']).len();
    let command = Statement {
        location: location(start, start + text.len()),
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
        ),
        condition: Condition::Always,
        negated: false,
        command: Command::Function {
            name: name.to_owned(),
            definition: Rc::new(super::FunctionDefinition {
                body: vec![command],
                location: location(open, line.len()),
            }),
        },
    })
}
