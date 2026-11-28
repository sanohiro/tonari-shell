use super::{Evaluation, Failure, Result, Variables, calculate, check_depth};
use crate::word::{Alternative, Modification, Part, Word, is_identifier};

pub(crate) fn data_key(source: &str, vars: &mut dyn Variables) -> Result<String> {
    data_key_at(source, vars, 0).map_err(Failure::message)
}

fn single_data_parameter(source: &str) -> Option<&str> {
    let parameter = source.strip_prefix('$')?;
    let name = parameter
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(parameter);
    crate::word::is_identifier(name).then_some(name)
}

pub(super) fn data_key_at(
    source: &str,
    vars: &mut dyn Variables,
    depth: u16,
) -> Evaluation<String> {
    if !source.is_empty() && !source.contains(['$', '`', '\'', '"', '\\']) {
        return Ok(source.to_owned());
    }
    let key = if let Some(name) = single_data_parameter(source) {
        data_parameter(vars, name, depth + 1)?
    } else {
        let word = crate::parser::subscript_word(source, true).map_err(|e| e.message())?;
        data_text(word, vars, depth + 1)?
    };
    if key.is_empty() {
        Err("bad array subscript".into())
    } else {
        Ok(key)
    }
}

pub(super) fn data_index_at(source: &str, vars: &mut dyn Variables, depth: u16) -> Evaluation<i64> {
    if !source.contains(['$', '`', '"', '\\']) {
        return calculate(source, vars, depth);
    }
    // Read a stored array count directly when it is the entire numeric
    // subscript. Compound expressions still go through the arithmetic parser.
    if let Some(operand) = source.strip_prefix("${#").and_then(|s| s.strip_suffix('}'))
        && let Some((base, _, false)) = crate::word::array_elements_name(operand)
        && let Some(count) = vars.session().referenced_array_len(base)
    {
        return Ok(count as i64);
    }
    if let Some(name) = single_data_parameter(source) {
        if let Some(n) = vars.decimal(name) {
            return Ok(n);
        }
        let value = data_parameter(vars, name, depth + 1)?;
        return calculate(&value, vars, depth);
    }
    let (keys, tail) = if source.contains('[') {
        crate::parser::arithmetic_segments(source, |name| vars.is_associative(name))
    } else {
        crate::parser::arithmetic_word(source).map(|word| (Vec::new(), word))
    }
    .map_err(|e| e.message())?;
    if keys.is_empty() {
        return calculate(&data_text(tail, vars, depth + 1)?, vars, depth);
    }
    let mut text = String::new();
    for (prefix, key) in keys {
        text.push_str(&data_text(prefix, vars, depth + 1)?);
        crate::word::quote::push_single_quote(&mut text, &data_text(key, vars, depth + 1)?);
    }
    text.push_str(&data_text(tail, vars, depth + 1)?);
    calculate(&text, vars, depth)
}

pub(crate) fn data_index(source: &str, vars: &mut dyn Variables) -> Result<i64> {
    data_index_at(source, vars, 0).map_err(Failure::message)
}

// Raw arithmetic targets already contain a decoded key. Only decode an index
// here when the spelling comes from the value of a name reference.
pub(super) fn read_name(
    name: &str,
    vars: &mut dyn Variables,
    depth: u16,
) -> Evaluation<Option<String>> {
    if vars.visible_nameref(name)
        && let std::borrow::Cow::Owned(target) = vars.session().reference_name(name)?
        && let Some((base, index)) = crate::session::array::subscript(&target)
    {
        return element(base, index, vars, depth);
    }
    Ok(vars.get(name)?)
}

fn element(
    base: &str,
    source: &str,
    vars: &mut dyn Variables,
    depth: u16,
) -> Evaluation<Option<String>> {
    check_depth(depth)?;
    if vars.is_associative(base) {
        let key = data_key_at(source, vars, depth + 1)?;
        Ok(vars.session().assoc_value(base, &key).map(str::to_owned))
    } else {
        let n = data_index_at(source, vars, depth + 1)?;
        let index = vars.session().relative_index(base, n)?;
        Ok(vars.session().element(base, index))
    }
}

fn value(name: &str, vars: &mut dyn Variables, depth: u16) -> Evaluation<Option<String>> {
    check_depth(depth)?;
    if name == "#" {
        return Ok(Some(vars.session().positional.len().to_string()));
    }
    if let Some(operand) = name.strip_prefix('#') {
        let count = if matches!(operand, "@" | "*") {
            vars.session().positional.len()
        } else if let Some((base, _, _)) = crate::word::array_elements_name(operand) {
            vars.session()
                .referenced_array_len(base)
                .or_else(|| {
                    let session = vars.session();
                    let indices = session.indices(base);
                    (!indices.is_empty() || session.initialized_array(base))
                        .then_some(indices.len())
                })
                .or_else(|| (!vars.nounset()).then_some(0))
                .ok_or_else(|| Failure::Unbound(operand.to_owned()))?
        } else {
            let s = value(operand, vars, depth + 1)?;
            if s.is_none()
                && vars.nounset()
                && !crate::session::array::subscript(operand)
                    .is_some_and(|(base, _)| vars.session().initialized_array(base))
            {
                return Err(Failure::Unbound(operand.to_owned()));
            }
            s.as_deref().map_or(0, crate::word::character_count)
        };
        return Ok(Some(count.to_string()));
    }
    if is_identifier(name) {
        return read_name(name, vars, depth + 1);
    }
    if let Some((base, index)) = crate::session::array::subscript(name)
        && is_identifier(base)
        && !matches!(index, "@" | "*")
    {
        return element(base, index, vars, depth + 1);
    }
    if name == "0" {
        return Ok(Some(vars.session().arg0.clone()));
    }
    if let Ok(index) = name.parse::<usize>()
        && index > 0
    {
        return Ok(vars.session().positional.as_slice().get(index - 1).cloned());
    }
    Err("invalid indirect array subscript".into())
}

fn data_parameter(vars: &mut dyn Variables, name: &str, depth: u16) -> Evaluation<String> {
    match value(name, vars, depth)? {
        Some(value) => Ok(value),
        None if vars.nounset() => Err(Failure::Unbound(name.to_owned())),
        None => Ok(String::new()),
    }
}

fn data_text(word: Word, vars: &mut dyn Variables, depth: u16) -> Evaluation<String> {
    Ok(resolved_word(word, vars, depth)?.scalar(&|_| None, &[]))
}

fn pattern(word: Word, vars: &mut dyn Variables, depth: u16) -> Evaluation<String> {
    let word = resolved_word(word, vars, depth)?;
    Ok(if vars.session().shopt.glob.extglob {
        word.extended_pattern(&|_| None, &[])
    } else {
        word.pattern(&|_| None, &[])
    })
}

// Resolve only data operations. Preserve quote markers until the consuming
// pattern or replacement decides which characters are literal.
fn resolved_word(mut word: Word, vars: &mut dyn Variables, depth: u16) -> Evaluation<Word> {
    check_depth(depth)?;
    let parts = match &mut word {
        Word::Literal(_) => return Ok(word),
        Word::Expanded(parts) => parts,
        _ => return Err("invalid indirect array subscript".into()),
    };
    for part in parts {
        let (text, quoted) = match part {
            Part::Text(_) | Part::Pattern(_) | Part::Unquoted(_) => continue,
            Part::BraceParameter(name) => (data_parameter(vars, name, depth + 1)?, false),
            Part::Parameter { name, quoted } => (data_parameter(vars, name, depth + 1)?, *quoted),
            Part::Alternative {
                name,
                op: op @ (Alternative::Default | Alternative::Alternate),
                check_empty,
                word,
                quoted,
            } => {
                let v = value(name, vars, depth + 1)?.filter(|s| !*check_empty || !s.is_empty());
                let chosen = if (*op == Alternative::Default) == v.is_none() {
                    data_text(std::mem::take(word), vars, depth + 1)?
                } else if *op == Alternative::Default {
                    v.unwrap_or_default()
                } else {
                    String::new()
                };
                (chosen, *quoted)
            }
            Part::Arithmetic { expression, quoted } => (
                data_index_at(expression, vars, depth + 1)?.to_string(),
                *quoted,
            ),
            Part::Trim {
                name,
                pattern: operand,
                prefix,
                longest,
                quoted,
            } => {
                let text = data_parameter(vars, name, depth + 1)?;
                let pattern = pattern(std::mem::take(operand), vars, depth + 1)?;
                let _locale = crate::pattern::parameter_locale(vars.session(), &pattern);
                let compiled = crate::pattern::ParameterPattern::new(
                    &pattern,
                    vars.session().shopt.glob.extglob,
                    None,
                );
                (
                    crate::pattern::trim(text, &compiled, *prefix, *longest),
                    *quoted,
                )
            }
            Part::Modify {
                name,
                operation,
                quoted,
            } => {
                let text = data_parameter(vars, name, depth + 1)?;
                let transformed = match operation.as_mut() {
                    Modification::Case {
                        direction,
                        all,
                        pattern: operand,
                    } => {
                        let pat = pattern(std::mem::take(operand), vars, depth + 1)?;
                        let _locale = crate::pattern::parameter_locale(vars.session(), &pat);
                        let filter = crate::pattern::extended::Compiled::case_filter(
                            pat,
                            vars.session().shopt.glob.extglob,
                        );
                        crate::pattern::convert_chars(
                            text.into(),
                            *direction,
                            *all,
                            filter.as_ref(),
                            vars.session().character_locale(),
                        )
                    }
                    Modification::Replace {
                        pattern: operand,
                        replacement,
                        mode,
                    } => {
                        let pat = pattern(std::mem::take(operand), vars, depth + 1)?;
                        let replacement =
                            resolved_word(std::mem::take(replacement), vars, depth + 1)?;
                        let session = vars.session();
                        let amp = session.shopt.patsub_replacement;
                        let replacement = if amp {
                            replacement.replacement(&|_| None, &[])
                        } else {
                            replacement.scalar(&|_| None, &[])
                        };
                        let _locale = crate::pattern::parameter_locale(session, &pat);
                        let fold = session
                            .shopt
                            .nocasematch
                            .then(|| session.character_locale());
                        let compiled = crate::pattern::ParameterPattern::new(
                            &pat,
                            session.shopt.glob.extglob,
                            fold,
                        );
                        crate::word::substitute(text, &compiled, &replacement, *mode, amp)
                    }
                    Modification::Slice { offset, length } => {
                        let mut bound = |w| {
                            let text = data_text(w, vars, depth + 1)?;
                            calculate(&text, vars, depth + 1)
                        };
                        let start = bound(std::mem::take(offset))?;
                        let len = length.take().map(bound).transpose()?;
                        crate::word::substring(&text, start, len)
                            .map_err(|_| "negative substring length")?
                            .to_owned()
                    }
                    Modification::Quote => {
                        crate::word::quote::parameter_quote(&text, vars.session())
                    }
                    _ => return Err("invalid indirect array subscript".into()),
                };
                (transformed, *quoted)
            }
            _ => return Err("invalid indirect array subscript".into()),
        };
        *part = if quoted {
            Part::Text(text)
        } else {
            Part::Unquoted(text)
        };
    }
    Ok(word)
}
