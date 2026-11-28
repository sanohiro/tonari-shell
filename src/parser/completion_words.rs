use super::Error;
use super::lexer::{backquote, dollar, quoted};
use crate::word::{Part, Word, WordBuilder};

/// A registered -W list is split on IFS. Shell control operators are data here.
pub(crate) fn parse(text: &str, ifs: &str) -> Result<Vec<Word>, Error> {
    let mut words = Vec::new();
    let mut word = WordBuilder::default();
    let mut started = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if ifs.contains(c) {
            if started {
                words.push(std::mem::take(&mut word).finish());
                started = false;
            }
            continue;
        }
        match c {
            '\'' | '"' => quoted(&mut chars, c, &mut word)?,
            '$' => dollar(&mut chars, &mut word, false)?,
            '`' => backquote(&mut chars, &mut word, false)?,
            '\\' => {
                let next = chars.next().ok_or(Error::incomplete_message("unfinished completion escape"))?;
                if next == '\n' {
                    continue;
                }
                word.push(next);
            }
            '~' if !started => {
                let suffix = chars.as_str();
                let len = suffix
                    .find(|c| c == '/' || ifs.contains(c))
                    .unwrap_or(suffix.len());
                let tilde = format!("~{}", &suffix[..len]);
                word.part(Part::Tilde(tilde));
                chars = suffix[len..].chars();
            }
            _ => word.push(c),
        }
        started = true;
    }
    if started {
        words.push(word.finish());
    }
    Ok(words)
}
