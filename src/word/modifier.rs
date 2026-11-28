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
    Attributes,
    Assignment,
    Pairs {
        separate: bool,
    },
    Quote,
    Escapes,
    Case {
        direction: crate::pattern::CaseDirection,
        all: bool,
        pattern: Word,
    },
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
            return Ok(Self::Elements(
                env.session.positional.as_slice().to_vec(),
                name == "@",
            ));
        }
        if let Some((base, at, keys)) = array_elements_name(name) {
            if !env.shadows(base) {
                let mut values = Vec::new();
                if env.session.append_array_fields(base, keys, &mut values) {
                    return Ok(Self::Elements(values, at));
                }
            }
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

// Only ASCII blocks precede the first mixed block, so its start is a UTF-8
// boundary. A non-ASCII character near the end never forces a second full scan.
pub(crate) fn character_count(text: &str) -> usize {
    const BLOCK: usize = 1024;
    for (i, bytes) in text.as_bytes().chunks(BLOCK).enumerate() {
        if !bytes.is_ascii() {
            let ascii = i * BLOCK;
            return ascii + text[ascii..].chars().count();
        }
    }
    text.len()
}

pub(super) fn value(name: &str, env: &mut Expansion<'_>) -> Result<Option<String>, Error> {
    if let Some(key) = name.strip_prefix('#')
        && !env.shadows(key)
        && let Some(n) = env.session.stored_char_count(key)
    {
        return Ok(Some(itoa::Buffer::new().format(n).to_owned()));
    }
    if let Some(text) = name
        .strip_prefix('#')
        .and_then(|key| env.borrowed_scalar(key))
    {
        let length = character_count(text);
        return Ok(Some(itoa::Buffer::new().format(length).to_owned()));
    }
    if let Some(reference) = name.strip_prefix('#')
        && env.session.is_nameref(reference)
        && !env.shadows(reference)
    {
        let target = env.session.reference_name(reference)?;
        if let Some((base, _, _)) = array_elements_name(&target) {
            if let Some(count) = env.session.stored_array_len(base) {
                return Ok(Some(count.to_string()));
            }
        } else if crate::session::array::subscript(&target).is_some() {
            return Ok(value(&target, env)?.map(|s| s.chars().count().to_string()));
        }
    }
    if let Some((base, _, _)) = name.strip_prefix('#').and_then(array_elements_name)
        && let Some(count) = env.session.stored_array_len(base)
    {
        return Ok(Some(count.to_string()));
    }
    // A name reference to an indexed element must evaluate its subscript in
    // the live expansion, where arithmetic updates are visible to later words.
    let resolved;
    let name = if env.session.is_nameref(name)
        && !env.shadows(name)
        && let Ok(std::borrow::Cow::Owned(target)) = env.session.reference_name(name)
        && crate::session::array::subscript(&target).is_some()
    {
        resolved = target;
        &resolved
    } else {
        name
    };
    let expanded;
    let name = if let Some((base, index)) = crate::session::array::subscript(name) {
        if !matches!(index, "@" | "*") && env.session.is_associative(base.trim_start_matches('#')) {
            let key = env.key(index)?;
            let value = env.session.assoc_value(base.trim_start_matches('#'), &key);
            return Ok(if base.starts_with('#') {
                Some(value.map_or(0, |s| s.chars().count()).to_string())
            } else {
                value.map(str::to_owned)
            });
        }
        if !matches!(index, "@" | "*") && !index.bytes().all(|b| b.is_ascii_digit()) {
            let i = env.arithmetic(index)?;
            if !env.shadows(base)
                && crate::word::is_identifier(base)
                && !env.session.is_nameref(base)
                && let Ok(index) = env.session.relative_index(base, i)
            {
                return Ok(env.session.element(base, index));
            }
            expanded = format!("{base}[{i}]");
            &expanded
        } else {
            name
        }
    } else {
        name
    };
    Ok(parameter(
        name,
        &|k| env.lookup(k),
        env.session.positional.as_slice(),
    ))
}

fn as_number(word: &mut Word, env: &mut Expansion<'_>) -> Result<i64, Error> {
    if let Word::Literal(s) = word
        && let Some(n) = crate::Session::plain_decimal(s.trim_ascii())
    {
        return Ok(n);
    }
    let text = env.scalar(std::mem::take(word))?;
    env.arithmetic_value(&text)
}

pub(super) fn scalar_part(text: String, quoted: bool) -> Part {
    if quoted {
        Part::Text(text)
    } else {
        Part::Unquoted(text)
    }
}

// Attribute transformations describe the target variable of a nameref.
fn attribute_base<'a>(
    name: &'a str,
    elements: Option<(&'a str, bool, bool)>,
    session: &crate::Session,
) -> std::borrow::Cow<'a, str> {
    let base = elements
        .map(|(base, _, _)| base)
        .or_else(|| crate::session::array::subscript(name).map(|(base, _)| base))
        .unwrap_or(name);
    session.reference_name(base).unwrap_or_else(|e| {
        diagnostic!("warning: {base}: {e}");
        std::borrow::Cow::Borrowed(base)
    })
}

fn assignment(name: &str, quoted: bool, env: &mut Expansion<'_>) -> Result<Part, Error> {
    let elements = array_elements_name(name);
    let fields = if matches!(name, "@" | "*") {
        let args = env.session.positional.as_slice();
        if args.is_empty() {
            Vec::new()
        } else {
            ["set".to_owned(), "--".to_owned()]
                .into_iter()
                .chain(
                    args.iter()
                        .map(|s| super::quote::parameter_quote(s, env.session)),
                )
                .collect()
        }
    } else {
        let resolved = attribute_base(name, elements, env.session);
        let base = resolved.as_ref();
        if elements.is_some()
            && let Some(fields) = env.session.array_declaration_fields(base)
        {
            fields
        } else if let Some((flags, _)) = env.session.attributes(base) {
            let key = if elements.is_some() { base } else { name };
            let quoted_value = if let Some(s) = env.borrowed_scalar(key) {
                Some(super::quote::parameter_quote(s, env.session))
            } else {
                value(key, env)?.map(|s| super::quote::parameter_quote(&s, env.session))
            };
            let mut fields = Vec::new();
            if !flags.is_empty() {
                fields.extend(["declare".to_owned(), format!("-{flags}")]);
            }
            if let Some(text) = quoted_value {
                fields.push(format!("{base}={text}"));
            } else if !fields.is_empty() {
                fields.push(base.to_owned());
            }
            if elements.is_some() && !fields.is_empty() {
                return Ok(scalar_part(fields.join(" "), quoted));
            }
            fields
        } else {
            Vec::new()
        }
    };
    if name == "@" || elements.is_some_and(|(_, at, _)| at) {
        return Ok(Part::Elements {
            values: fields,
            at: true,
            quoted,
        });
    }
    Ok(scalar_part(fields.join(" "), quoted))
}

impl Modification {
    pub(super) fn evaluate(
        &mut self,
        name: &str,
        quoted: bool,
        env: &mut Expansion<'_>,
    ) -> Result<Part, Error> {
        let scalar = |s| scalar_part(s, quoted);
        match self {
            Self::Replace {
                pattern,
                replacement,
                mode,
            } => {
                // Dynamic operands may change the subject while expanding.
                let snapshot = (!(pattern.fixed_parts() && replacement.fixed_parts()))
                    .then(|| Values::read(name, env))
                    .transpose()?;
                let pattern = env.pattern(std::mem::take(pattern))?;
                replacement.resolve(env)?;
                let replacement = std::mem::take(replacement);
                let lookup = |k: &str| env.lookup(k);
                let positional = env.session.positional.as_slice();
                let replacement = if env.session.shopt.patsub_replacement {
                    replacement.replacement(&lookup, positional)
                } else {
                    replacement.scalar(&lookup, positional)
                };
                let compiled = crate::pattern::ParameterPattern::new(
                    &pattern,
                    env.session.shopt.glob.extglob,
                    env.session
                        .shopt
                        .nocasematch
                        .then(|| env.session.character_locale()),
                );
                if snapshot.is_none()
                    && compiled.fold.is_none()
                    && compiled.group.is_none()
                    && !pattern.is_empty()
                    && !crate::pattern::has_magic(&pattern, false)
                    && let Some(text) = env.borrowed_scalar(name)
                {
                    return Ok(scalar(super::replace::literal(
                        text,
                        &crate::pattern::unescape(&pattern),
                        &replacement,
                        *mode,
                        env.session.shopt.patsub_replacement,
                    )));
                }
                let values = match snapshot {
                    Some(values) => values,
                    None => Values::read(name, env)?,
                };
                Ok(values.map(quoted, |v| {
                    super::replace::substitute(
                        v,
                        &compiled,
                        &replacement,
                        *mode,
                        env.session.shopt.patsub_replacement,
                    )
                }))
            }
            Self::Case {
                direction,
                all,
                pattern,
            } => {
                let snapshot = (!pattern.fixed_parts())
                    .then(|| Values::read(name, env))
                    .transpose()?;
                let pattern = match std::mem::take(pattern) {
                    Word::Literal(text) if text.is_empty() => text,
                    word => env.pattern(word)?,
                };
                let filter = if pattern.is_empty() || pattern == "?" {
                    None
                } else {
                    Some(crate::pattern::Matcher::new(pattern.as_str()))
                };
                let locale = env.session.character_locale();
                let direction = *direction;
                let all = *all;
                // Bash applies the locale's single-character case conversion.
                // In particular an uppercase expansion never grows into two letters.
                if snapshot.is_none()
                    && let Some(text) = env.borrowed_scalar(name)
                {
                    return Ok(scalar(crate::pattern::convert_chars(
                        text.into(),
                        direction,
                        all,
                        filter.as_ref(),
                        locale,
                    )));
                }
                let values = match snapshot {
                    Some(v) => v,
                    None => Values::read(name, env)?,
                };
                Ok(values.map(quoted, |s| {
                    crate::pattern::convert_chars(s.into(), direction, all, filter.as_ref(), locale)
                }))
            }
            Self::Assignment => assignment(name, quoted, env),
            Self::Quote | Self::Pairs { .. } | Self::Escapes => {
                if let Self::Pairs { separate } = self
                    && let Some((base, at, false)) = array_elements_name(name)
                {
                    let base = attribute_base(base, None, env.session);
                    if let Some(values) = env.session.array_pair_fields(&base, *separate) {
                        return Ok(Part::Elements { values, at, quoted });
                    }
                }
                let transform = |text: &str, session: &crate::Session| -> Result<String, Error> {
                    if !matches!(self, Self::Escapes) {
                        Ok(super::quote::parameter_quote(text, session))
                    } else {
                        crate::parser::ansi_string(text).map_err(|e| Error::from(e.message()))
                    }
                };
                if let Some(text) = env.borrowed_scalar(name) {
                    return Ok(scalar(transform(text, env.session)?));
                }
                let values = if array_elements_name(name).is_some() || matches!(name, "@" | "*") {
                    Values::read(name, env)?
                } else if let Some(text) = value(name, env)? {
                    Values::Scalar(text)
                } else if env.session.nounset {
                    return Err(env.missing(name, "unbound variable"));
                } else {
                    return Ok(scalar(String::new()));
                };
                match values {
                    Values::Scalar(text) => Ok(scalar(transform(&text, env.session)?)),
                    Values::Elements(values, at) => Ok(Part::Elements {
                        values: values
                            .into_iter()
                            .map(|s| transform(&s, env.session))
                            .collect::<Result<_, _>>()?,
                        at,
                        quoted,
                    }),
                }
            }
            Self::Attributes => {
                if matches!(name, "@" | "*") {
                    return Ok(Part::Elements {
                        values: vec![String::new(); env.session.positional.as_slice().len()],
                        at: name == "@",
                        quoted,
                    });
                }
                let elements = array_elements_name(name);
                let resolved = attribute_base(name, elements, env.session);
                let base = resolved.as_ref();
                let (flags, assigned_array) = env.session.attributes(base).unwrap_or_default();
                let stored_len = env.session.stored_array_len(base);
                if env.session.nounset
                    && elements.is_none()
                    && !(crate::session::array::subscript(name).is_none()
                        && stored_len.is_some_and(|n| n > 0))
                {
                    value(name, env)?.ok_or_else(|| env.missing(name, "unbound variable"))?;
                }
                if let Some((_, at, _)) = elements {
                    let count = if let Some(len) = stored_len {
                        if !assigned_array && len == 0 && !flags.is_empty() {
                            1
                        } else {
                            len
                        }
                    } else if flags.is_empty() {
                        0
                    } else if let Values::Elements(values, _) = Values::read(name, env)? {
                        values.len().max(1)
                    } else {
                        unreachable!()
                    };
                    return Ok(Part::Elements {
                        values: vec![flags; count],
                        at,
                        quoted,
                    });
                }
                Ok(scalar(flags))
            }
            Self::Prompt => {
                let status = env.status();
                Ok(Values::read(name, env)?.map(quoted, |v| {
                    crate::prompt::render_text(env.session, v, status)
                }))
            }
            Self::Slice { offset, length } => {
                // Nonconstant bounds can assign to the parameter being sliced.
                // Snapshot ordinary scalars before either arithmetic expression.
                let constant = |word: &Word| word.as_literal().is_some_and(|s| crate::Session::plain_decimal(s.trim_ascii()).is_some());
                if !(constant(offset) && length.as_ref().is_none_or(constant))
                    && super::is_identifier(name)
                {
                    let before = value(name,env)?;
                    if env.session.nounset && before.is_none() { return Err(env.missing(name, "unbound variable")); }
                    let start = as_number(offset,env)?;
                    let len = length.as_mut().map(|w|as_number(w,env)).transpose()?;
                    return Ok(scalar(substring(before.as_deref().unwrap_or(""), start, len)?.to_owned()));
                }
                let start = as_number(offset, env)?;
                let len = length.as_mut().map(|w| as_number(w, env)).transpose()?;
                if let Some((base, _, false)) = array_elements_name(name) {
                    if let Some(text) = env.borrowed_scalar(base) {
                        return Ok(scalar(substring(text, start, len)?.to_owned()));
                    }
                    let target = env.session.reference_name(base)?;
                    if !env.session.array_parameter(&target)
                        && let Some(text) = env.lookup(&target)
                    {
                        return Ok(scalar(substring(&text, start, len)?.to_owned()));
                    }
                }
                if matches!(name, "@" | "*") || array_elements_name(name).is_some() {
                    if len.is_some_and(|n| n < 0) {
                        return Err(Error::discard_line("negative array slice length"));
                    }
                    let count = len.map_or(usize::MAX, |n| n as usize);
                    let (values, at) = if let Some((base, at, keys)) = array_elements_name(name) {
                        if !keys
                            && !env.has_pending_assignments()
                            && let Some(values) =
                                env.session.stored_indexed_slice(base, start, count)
                        {
                            return Ok(Part::Elements { values, at, quoted });
                        }
                        if env.session.is_associative(base) {
                            let values = super::array_elements(base, keys, &|k| env.lookup(k))
                                .unwrap_or_default();
                            let offset = if start < 0 {
                                (values.len() as i64).saturating_add(start)
                            } else {
                                start.saturating_sub(1).max(0)
                            };
                            let values = if offset < 0 {
                                Vec::new()
                            } else {
                                values
                                    .into_iter()
                                    .skip(offset as usize)
                                    .take(count)
                                    .collect()
                            };
                            return Ok(Part::Elements { values, at, quoted });
                        }
                        let target = env.session.reference_name(base)?;
                        let indexes = env.session.indices(&target);
                        let start = crate::session::array::absolute_index(start, || {
                            indexes.last().copied()
                        });
                        let values = start.map_or_else(Vec::new, |start| {
                            let begin = indexes.partition_point(|&i| i < start);
                            indexes[begin..]
                                .iter()
                                .take(count)
                                .filter_map(|&i| {
                                    if keys {
                                        Some(i.to_string())
                                    } else if env.has_pending_assignments() {
                                        env.lookup(&format!("{base}[{i}]"))
                                    } else {
                                        env.session.element(&target, i)
                                    }
                                })
                                .collect()
                        });
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
                                .chain(env.session.positional.as_slice().iter())
                                .skip(start as usize)
                                .take(count)
                                .cloned()
                                .collect()
                        };
                        (values, name == "@")
                    };
                    return Ok(Part::Elements { values, at, quoted });
                }
                if let Some(text) = env.borrowed_scalar(name) {
                    return Ok(scalar(substring(text, start, len)?.to_owned()));
                }
                let v = value(name, env)?;
                if env.session.nounset && v.is_none() {
                    return Err(env.missing(name, "unbound variable"));
                }
                let v = v.unwrap_or_default();
                let result = substring(&v, start, len)?;
                Ok(scalar(if result.len() == v.len() {
                    v
                } else {
                    result.to_owned()
                }))
            }
        }
    }
}

fn substring(v: &str, start: i64, len: Option<i64>) -> Result<&str, Error> {
    // Positive bounds only need the requested prefix. Negative bounds
    // count from the far end, independent of the UTF-8 byte width.
    let boundary = |s: &str, n: i64| {
        let magnitude = n.unsigned_abs() as usize;
        if magnitude > s.len() {
            return None;
        }
        let bytes = s.as_bytes();
        let part = if n < 0 {
            &bytes[s.len() - magnitude..]
        } else {
            &bytes[..magnitude]
        };
        if part.is_ascii() {
            return Some(if n < 0 {
                s.len() - magnitude
            } else {
                magnitude
            });
        }
        let mut rest = s;
        let mut left = magnitude;
        // chars().count() counts UTF-8 lead bytes in machine words. Consume
        // whole chunks before resolving the final character boundary.
        const BLOCK: usize = 1024;
        while left >= BLOCK && rest.len() >= BLOCK {
            if n < 0 {
                let mut cut = rest.len() - BLOCK;
                while !rest.is_char_boundary(cut) {
                    cut += 1;
                }
                left -= rest[cut..].chars().count();
                rest = &rest[..cut];
            } else {
                let mut cut = BLOCK;
                while !rest.is_char_boundary(cut) {
                    cut -= 1;
                }
                left -= rest[..cut].chars().count();
                rest = &rest[cut..];
            }
        }
        if left == 0 {
            return Some(if n < 0 {
                rest.len()
            } else {
                s.len() - rest.len()
            });
        }
        if n < 0 {
            rest.char_indices().rev().nth(left - 1).map(|(i, _)| i)
        } else {
            rest.char_indices()
                .nth(left - 1)
                .map(|(i, c)| s.len() - rest.len() + i + c.len_utf8())
        }
    };
    let Some(start) = boundary(v, start) else {
        return Ok("");
    };
    let end = match len {
        None => v.len(),
        Some(n) if n < 0 => boundary(v, n)
            .filter(|&end| end >= start)
            .ok_or_else(|| Error::discard_line("negative substring length"))?,
        Some(n) => boundary(&v[start..], n).map_or(v.len(), |i| start + i),
    };
    Ok(&v[start..end])
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
