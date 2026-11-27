use super::*;

impl Variable {
    pub(super) fn attribute_flags(&self) -> impl Iterator<Item = u8> {
        [
            (self.associative().is_some(), b'A'),
            (
                self.elements.is_some() && self.associative().is_none(),
                b'a',
            ),
            (self.integer, b'i'),
            (self.nameref, b'n'),
            (self.readonly, b'r'),
            (self.exported, b'x'),
            (self.letter_case == Some(LetterCase::Lower), b'l'),
            (self.letter_case == Some(LetterCase::Upper), b'u'),
            (self.letter_case == Some(LetterCase::Capitalize), b'c'),
        ]
        .into_iter()
        .filter_map(|(on, flag)| on.then_some(flag))
    }
}

impl Session {
    // Stored arrays can be formatted in place. Dynamic shell arrays are materialized
    // on demand so each transform sees the current snapshot.
    fn transform_array(&self, name: &str) -> Option<Cow<'_, Variable>> {
        if let Some(var) = self.variables.get(name)
            && !var.dynamic
        {
            return var.elements.as_ref().map(|_| Cow::Borrowed(var));
        }
        if !self.array_parameter(name) {
            return None;
        }
        self.declared_variable(name, false)
            .filter(|v| v.elements.is_some())
            .map(Cow::Owned)
    }

    pub(crate) fn array_declaration_fields(&self, name: &str) -> Option<Vec<String>> {
        let v = self.transform_array(name)?;
        let mut assignment = name.as_bytes().to_vec();
        print_variable_value(&mut assignment, &v).expect("in-memory write");
        assignment.pop();
        Some(vec![
            "declare".into(),
            format!(
                "-{}",
                v.attribute_flags().map(char::from).collect::<String>()
            ),
            String::from_utf8(assignment).unwrap(),
        ])
    }

    pub(crate) fn array_pair_fields(&self, name: &str, separate: bool) -> Option<Vec<String>> {
        let var = self.transform_array(name)?;
        let mut fields = Vec::new();
        let mut text = Vec::new();
        let mut add = |key: &str, value: &str| {
            if separate {
                fields.extend([key.to_owned(), value.to_owned()]);
            } else {
                if !text.is_empty() {
                    text.push(b' ');
                }
                if key
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_-./".contains(c))
                    && !key.is_empty()
                {
                    text.extend_from_slice(key.as_bytes());
                } else {
                    print_value(&mut text, key).expect("in-memory write");
                }
                text.push(b' ');
                print_value(&mut text, value).expect("in-memory write");
            }
        };
        if let Some(map) = var.associative() {
            for (key, value) in map.ordered() {
                add(key, value);
            }
            if !text.is_empty() {
                text.push(b' ');
            }
        } else if let Some(array) = var.elements.as_deref() {
            for (index, value) in array.iter() {
                add(&index.to_string(), value);
            }
        }
        if !text.is_empty() {
            fields.push(String::from_utf8(text).unwrap());
        }
        Some(fields)
    }

    pub(crate) fn attributes(&self, name: &str) -> Option<(String, bool)> {
        // An array member has a value but no declaration attributes of its own.
        if array::subscript(name).is_some() {
            return None;
        }
        if let Some(var) = self.variables.get(name) {
            if var.nameref {
                return None;
            }
            return Some((
                var.attribute_flags().map(char::from).collect(),
                var.array_assigned,
            ));
        }
        self.declared_variable(name, false).map(|v| {
            (
                v.attribute_flags().map(char::from).collect(),
                v.array_assigned,
            )
        })
    }
}

pub(super) fn print_attribute(
    out: &mut dyn Write,
    name: &str,
    var: &Variable,
    command: Option<&str>,
) -> io::Result<()> {
    if let Some(command) = command {
        out.write_all(command.as_bytes())?;
        if var.associative().is_some() {
            out.write_all(b" -A")?;
        } else if var.elements.is_some() {
            out.write_all(b" -a")?;
        }
        write!(out, " {name}")?;
        return print_variable_value(out, var);
    }
    print_declaration(out, name, var)
}

pub(super) fn print_declaration(out: &mut dyn Write, name: &str, var: &Variable) -> io::Result<()> {
    write!(out, "declare -")?;
    let mut flags = var.attribute_flags().peekable();
    if flags.peek().is_none() {
        out.write_all(b"-")?;
    }
    for flag in flags {
        out.write_all(&[flag])?;
    }
    write!(out, " {name}")?;
    print_variable_value(out, var)
}

pub(super) fn set_value(value: &str, posix: bool, locale: CharacterLocale) -> Cow<'_, str> {
    let ascii = matches!(locale, CharacterLocale::Ascii);
    if !posix
        && value
            .bytes()
            .any(|c| c.is_ascii_control() || ascii && !c.is_ascii())
    {
        return String::from_utf8(crate::word::quote::ansi_quote(value.as_bytes(), ascii))
            .unwrap()
            .into();
    }
    if value == "'" {
        return Cow::Borrowed("\\'");
    }
    if value.starts_with(['#', '~'])
        || value.bytes().any(|c| {
            c.is_ascii_whitespace()
                || matches!(
                    c,
                    b'!' | b'$'
                        | b'&'
                        | b'*'
                        | b'('
                        | b')'
                        | b'{'
                        | b'}'
                        | b'['
                        | b']'
                        | b';'
                        | b'<'
                        | b'>'
                        | b'?'
                        | b'\\'
                        | b'"'
                        | b'\''
                        | b'|'
                        | 0x60
                )
        })
    {
        crate::word::quote::single_quote(value).into()
    } else {
        Cow::Borrowed(value)
    }
}

pub(super) fn print_variable_value(out: &mut dyn Write, var: &Variable) -> io::Result<()> {
    if let Some(map) = var.associative() {
        if var.array_assigned {
            write!(out, "=(")?;
            for (key, value) in map.ordered() {
                write!(out, "[")?;
                if key
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_-./".contains(c))
                {
                    write!(out, "{key}")?;
                } else {
                    print_value(out, key)?;
                }
                write!(out, "]=")?;
                print_value(out, value)?;
                write!(out, " ")?;
            }
            write!(out, ")")?;
        }
    } else if let Some(array) = &var.elements {
        if var.array_assigned {
            write!(out, "=(")?;
            for (position, (index, value)) in array.iter().enumerate() {
                if position > 0 {
                    write!(out, " ")?;
                }
                write!(out, "[{index}]=")?;
                print_value(out, value)?;
            }
            write!(out, ")")?;
        }
    } else if let Some(value) = &var.value {
        write!(out, "=")?;
        print_value(out, value)?;
    }
    writeln!(out)
}

pub(super) fn print_value(out: &mut dyn Write, value: &str) -> io::Result<()> {
    if value.bytes().any(|c| c.is_ascii_control()) {
        return out.write_all(&crate::word::quote::ansi_quote(value.as_bytes(), false));
    }
    out.write_all(b"\"")?;
    let mut start = 0;
    for (i, &byte) in value.as_bytes().iter().enumerate() {
        if matches!(byte, b'"' | b'\\' | b'$' | b'\x60') {
            out.write_all(&value.as_bytes()[start..i])?;
            out.write_all(b"\\")?;
            start = i;
        }
    }
    out.write_all(&value.as_bytes()[start..])?;
    out.write_all(b"\"")
}
