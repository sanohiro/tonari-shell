use super::{Part, Word, array_elements_name, parameter};
use crate::expansion::{Error, Expansion};

#[derive(Clone, Debug)]
pub enum Modification {
    Replace {
        pattern: Word,
        replacement: Word,
        mode: ReplaceMode,
    },
    Slice {
        offset: Word,
        length: Option<Word>,
    },
    Prompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceMode {
    First,
    All,
    Prefix,
    Suffix,
}

pub(super) enum Values {
    Scalar(String),
    Elements(Vec<String>, bool),
}
impl Values {
    pub(super) fn read(name: &str, env: &mut Expansion<'_>) -> Result<Self, Error> {
        if matches!(name, "@" | "*") {
            return Ok(Self::Elements(env.session.positional.clone(), name == "@"));
        }
        if let Some((base, at, keys)) = array_elements_name(name) {
            return Ok(Self::Elements(
                super::array_elements(base, keys, &|k| env.lookup(k)).unwrap_or_default(),
                at,
            ));
        }
        let v = value(name, env)?;
        if v.is_none() && env.session.nounset {
            return Err(env.missing(name, "unbound variable"));
        }
        Ok(Self::Scalar(v.unwrap_or_default()))
    }
    pub(super) fn map(self, quoted: bool, mut f: impl FnMut(String) -> String) -> Part {
        match self {
            Self::Scalar(s) => {
                if quoted {
                    Part::Text(f(s))
                } else {
                    Part::Unquoted(f(s))
                }
            }
            Self::Elements(elements, at) => Part::Elements {
                values: elements.into_iter().map(f).collect(),
                at,
                quoted,
            },
        }
    }
}

pub(super) fn value(name: &str, env: &mut Expansion<'_>) -> Result<Option<String>, Error> {
    let expanded;
    let name = if let Some((base, index)) = crate::session::array::subscript(name) {
        if !matches!(index, "@" | "*") && index.contains(['$', '`']) {
            let i = env.arithmetic(index)?;
            expanded = format!("{base}[{i}]");
            &expanded
        } else {
            name
        }
    } else {
        name
    };
    Ok(parameter(name, &|k| env.lookup(k), &env.session.positional))
}

fn as_number(word: &mut Word, env: &mut Expansion<'_>) -> Result<i64, Error> {
    let text = env.scalar(std::mem::replace(word, Word::Literal(String::new())))?;
    env.arithmetic(&text)
}

impl Modification {
    pub(super) fn evaluate(
        &mut self,
        name: &str,
        quoted: bool,
        env: &mut Expansion<'_>,
    ) -> Result<Part, Error> {
        let scalar = |s| {
            if quoted {
                Part::Text(s)
            } else {
                Part::Unquoted(s)
            }
        };
        match self {
            Self::Replace {
                pattern,
                replacement,
                mode,
            } => {
                let values = Values::read(name, env)?;
                let pattern =
                    env.pattern(std::mem::replace(pattern, Word::Literal(String::new())))?;
                replacement.resolve(env)?;
                let replacement = std::mem::replace(replacement, Word::Literal(String::new()))
                    .replacement(&|k| env.lookup(k), &env.session.positional);
                Ok(values.map(quoted, |v| {
                    super::replace::substitute(
                        v,
                        &pattern,
                        &replacement,
                        *mode,
                        env.session.shopt.patsub_replacement,
                    )
                }))
            }
            Self::Prompt => {
                let prompt = value(name, env)?.unwrap_or_default();
                let status = env.status();
                Ok(scalar(crate::prompt::render_text(
                    env.session,
                    prompt,
                    status,
                )))
            }
            Self::Slice { offset, length } => {
                let start = as_number(offset, env)?;
                let len = length.as_mut().map(|w| as_number(w, env)).transpose()?;
                if matches!(name, "@" | "*") || array_elements_name(name).is_some() {
                    if len.is_some_and(|n| n < 0) {
                        return Err(Error::fatal("negative array slice length", 1));
                    }
                    let count = len.map_or(usize::MAX, |n| n as usize);
                    let (values, at) = if let Some((base, at, keys)) = array_elements_name(name) {
                        let indexes = env.lookup(&format!("\u{1}{base}")).unwrap_or_default();
                        let last = indexes
                            .split_ascii_whitespace()
                            .next_back()
                            .and_then(|i| i.parse::<i64>().ok())
                            .unwrap_or(-1);
                        let start = if start < 0 {
                            last.saturating_add(1).saturating_add(start)
                        } else {
                            start
                        };
                        let values = if start < 0 {
                            Vec::new()
                        } else {
                            indexes
                                .split_ascii_whitespace()
                                .filter(|i| i.parse::<i64>().is_ok_and(|i| i >= start))
                                .take(count)
                                .filter_map(|i| {
                                    if keys {
                                        Some(i.to_owned())
                                    } else {
                                        env.lookup(&format!("{base}[{i}]"))
                                    }
                                })
                                .collect()
                        };
                        (values, at)
                    } else {
                        let start = if start < 0 {
                            (env.session.positional.len() as i64 + 1).saturating_add(start)
                        } else {
                            start
                        };
                        let values = if start < 0 {
                            Vec::new()
                        } else {
                            std::iter::once(&env.session.arg0)
                                .chain(env.session.positional.iter())
                                .skip(start as usize)
                                .take(count)
                                .cloned()
                                .collect()
                        };
                        (values, name == "@")
                    };
                    return Ok(Part::Elements { values, at, quoted });
                }
                let v = value(name, env)?;
                if env.session.nounset && v.is_none() {
                    return Err(env.missing(name, "unbound variable"));
                }
                let v = v.unwrap_or_default();
                let total = v.chars().count() as i64;
                let start = if start < 0 {
                    total.saturating_add(start)
                } else {
                    start
                };
                if !(0..=total).contains(&start) {
                    return Ok(scalar(String::new()));
                }
                let end = match len {
                    None => total,
                    Some(n) if n < 0 => total.saturating_add(n),
                    Some(n) => start.saturating_add(n).min(total),
                };
                if end < start {
                    return Err(Error::fatal("negative substring length", 1));
                }
                Ok(scalar(
                    v.chars()
                        .skip(start as usize)
                        .take((end - start) as usize)
                        .collect(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Command, parse};

    fn expand(source: &str, env: &mut Expansion<'_>) -> Vec<String> {
        let Command::Pipeline(mut pipeline) = parse(source).unwrap().remove(0).command else {
            panic!()
        };
        pipeline
            .remove(0)
            .words
            .into_iter()
            .flat_map(|w| env.fields(w).unwrap())
            .collect()
    }

    #[test]
    fn scalar_slices_use_character_offsets_and_array_slices_keep_fields() {
        let mut session = crate::Session::default();
        session.assign("v", "é🌸abcdef");
        session.assign_element("a", 2, "first".into());
        session.assign_element("a", 5, "two words".into());
        session.assign_element("a", 8, "".into());
        let captured = std::cell::Cell::new(None);
        let mut env = Expansion::new(&mut session, 0, &captured);
        assert_eq!(
            expand(r#"printf "${v:$((${#v}-7)):2}" "${a[@]:3:2}" "#, &mut env),
            ["printf", "🌸a", "two words", ""]
        );
    }

    #[test]
    fn replacement_keeps_pattern_quotes_and_expands_only_unquoted_ampersands() {
        let mut session = crate::Session::default();
        session.assign("v", "axab");
        let captured = std::cell::Cell::new(None);
        let mut env = Expansion::new(&mut session, 0, &captured);
        assert_eq!(
            expand(
                r#"p "${v//a/[&]}" "${v/#a/'&'}" "${v/%b/end}" "${v/a?/-}" "#,
                &mut env
            ),
            ["p", "[a]x[a]b", "&xab", "axaend", "-ab"]
        );
    }

    #[test]
    fn trims_every_array_element_independently() {
        let mut session = crate::Session::default();
        session.replace_array(
            "a",
            vec!["issue\tIssue details".into(), "pr\tReview".into()],
        );
        session.assign("tab", "\t");
        let captured = std::cell::Cell::new(None);
        let mut env = Expansion::new(&mut session, 0, &captured);
        assert_eq!(
            expand(r#"p "${a[@]%%$tab*}" "#, &mut env),
            ["p", "issue", "pr"]
        );
    }
}
