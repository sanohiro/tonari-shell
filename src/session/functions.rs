use super::{Function, Session};
use crate::parser::{Command, FunctionDefinition, is_function_name, parse_final};
use std::{
    io::{self, Write},
    rc::Rc,
};

#[derive(Clone, Default)]
pub(crate) struct FunctionTable {
    entries: foldhash::HashMap<String, Function>,
    names: std::cell::OnceCell<Vec<String>>,
}

impl std::ops::Deref for FunctionTable {
    type Target = foldhash::HashMap<String, Function>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl FunctionTable {
    fn insert(&mut self, name: String, function: Function) {
        if self.entries.insert(name, function).is_none() {
            self.names.take();
        }
    }

    fn remove(&mut self, name: &str) {
        if self.entries.remove(name).is_some() {
            self.names.take();
        }
    }

    pub(crate) fn names(&self, prefix: &str) -> impl Iterator<Item = &String> {
        let names = self.names.get_or_init(|| {
            let mut names: Vec<_> = self.entries.keys().cloned().collect();
            names.sort_unstable();
            names
        });
        let start = if prefix.is_empty() {
            0
        } else {
            names.partition_point(|name| name.as_str() < prefix)
        };
        names[start..]
            .iter()
            .take_while(move |name| name.starts_with(prefix))
    }
}

// Keep the encoded child-environment key and its optional removal in one
// entry. Shell-side attribute queries use the original function name directly.
#[derive(Clone)]
pub(super) struct Export {
    pub(super) key: String,
    pub(super) body: Option<String>,
}

impl Function {
    fn export_value(&self) -> String {
        let body = self.definition.location.text();
        if body.starts_with('{') && body.ends_with('}') {
            format!("() {body}")
        } else {
            format!("() {{\n{body}\n}}")
        }
    }
}

impl Session {
    pub fn function_nesting_full(&self) -> bool {
        static INHERITED: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let bound = match self.variables.get("FUNCNEST") {
            Some(var) if var.nameref || var.elements.is_some() => self
                .get("FUNCNEST")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            Some(var) => var
                .value
                .as_deref()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            None => *INHERITED.get_or_init(|| {
                std::env::var("FUNCNEST")
                    .ok()
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0)
            }),
        };
        bound > 0 && self.function_depth - self.substitution_scopes >= bound
            || crate::stack_budget_exceeded()
    }

    pub(crate) fn push_source_path(&mut self, path: &std::path::Path) {
        let absolute = self
            .shopt
            .bash_source_fullpath
            .then(|| path.canonicalize().ok())
            .flatten();
        self.source_names.push(
            absolute
                .as_deref()
                .unwrap_or(path)
                .to_string_lossy()
                .as_ref()
                .into(),
        );
    }

    pub(crate) fn diagnostic_prefix(&self, line: impl FnOnce() -> usize) -> Option<String> {
        self.shopt
            .gnu_errfmt
            .then(|| format!("{}:{}: ", self.function_source(), line()))
    }

    pub(crate) fn function_source(&self) -> &str {
        self.source_names
            .last()
            .map(AsRef::as_ref)
            .unwrap_or_else(|| if self.interactive { "main" } else { &self.arg0 })
    }

    pub(crate) fn define_function(&mut self, name: String, definition: Rc<FunctionDefinition>) {
        let source = self.source_names.last().cloned().unwrap_or_else(|| {
            let default = if self.interactive { "main" } else { &self.arg0 };
            let cached = self
                .default_function_source
                .get_or_insert_with(|| Rc::from(default));
            if cached.as_ref() != default {
                *cached = Rc::from(default);
            }
            cached.clone()
        });
        let function = Function { definition, source };
        if self.allexport || self.function_exported(&name) {
            self.set_function_export(&name, Some(function.export_value()));
        }
        self.functions.insert(name, function);
    }

    pub(crate) fn import_function(&mut self, entry: &[u8]) {
        let Some(rest) = entry.strip_prefix(b"BASH_FUNC_") else {
            return;
        };
        let Ok(rest) = std::str::from_utf8(rest) else {
            return;
        };
        let Some((name, value)) = rest.split_once("%%=") else {
            return;
        };
        if !is_function_name(name) || !value.starts_with("() {") {
            return;
        }
        if name.contains('/') {
            diagnostic!("error importing function definition for `{name}'");
            return;
        }
        self.set_function_export(name, None);
        // Parse only the declaration body supplied by the protocol.
        // The environment key stays a name throughout the import.
        let Ok(mut parsed) = parse_final(&format!("__tonari_import {value}")) else {
            return;
        };
        if parsed.len() != 1 {
            return;
        }
        let statement = parsed.pop().unwrap();
        if statement.negated {
            return;
        }
        if let Command::Function {
            name: actual,
            definition,
        } = statement.command
            && actual == "__tonari_import"
        {
            self.functions.insert(
                name.to_owned(),
                Function {
                    definition,
                    source: "environment".into(),
                },
            );
            self.set_function_export(name, Some(value.to_owned()));
        }
    }

    pub fn export_function(&mut self, name: &str, on: bool) -> Result<(), &'static str> {
        let function = self.functions.get(name).ok_or("not a function")?;
        let value = on.then(|| function.export_value());
        self.set_function_export(name, value);
        Ok(())
    }

    fn set_function_export(&mut self, name: &str, body: Option<String>) {
        if let Some(export) = self.function_environment.get_mut(name) {
            export.body = body;
        } else {
            self.function_environment.insert(
                name.to_owned(),
                Export {
                    key: format!("BASH_FUNC_{name}%%"),
                    body,
                },
            );
        }
    }

    pub fn function_exported(&self, name: &str) -> bool {
        self.function_environment
            .get(name)
            .is_some_and(|export| export.body.is_some())
    }

    pub fn forget_function(&mut self, name: &str) {
        if self.function_exported(name) {
            let _ = self.export_function(name, false);
        }
        self.traced_functions.remove(name);
        self.functions.remove(name);
    }

    pub fn function_attributes(&self, name: &str) -> &'static str {
        match (
            self.readonly_functions.contains(name),
            self.traced_functions.contains(name),
            self.function_exported(name),
        ) {
            (false, false, false) => "f",
            (false, false, true) => "fx",
            (false, true, false) => "ft",
            (false, true, true) => "ftx",
            (true, false, false) => "fr",
            (true, false, true) => "frx",
            (true, true, false) => "frt",
            (true, true, true) => "frtx",
        }
    }

    pub fn print_function(
        &self,
        out: &mut dyn Write,
        name: &str,
        attributes: bool,
    ) -> io::Result<()> {
        let flags = if attributes {
            self.function_attributes(name)
        } else {
            "f"
        };
        self.functions[name].print(out, name, flags)
    }
}

impl Function {
    pub(super) fn print(&self, out: &mut dyn Write, name: &str, flags: &str) -> io::Result<()> {
        let definition = &self.definition;
        let body = definition.location.text();
        if crate::parser::reserved(name)
            || name
                .split_once('=')
                .is_some_and(|(key, _)| crate::parser::is_identifier(key.trim_end_matches('+')))
        {
            write!(out, "function ")?;
        }
        let rendered = definition.rendered_body.get_or_init(|| {
            let statements = definition.body.statements();
            let simple = body.starts_with('{') && body.ends_with('}') && statements.iter().all(|s|
                s.condition == crate::parser::Condition::Always && !s.negated
                && matches!(&s.command, Command::Pipeline(stages) if stages.iter().all(|c| c.compound.is_none() && c.redirects.is_empty()))
                && !s.location.text().contains('\n'));
            if !simple { return body.to_owned(); }
            let mut result = String::with_capacity(body.len() + 4 * statements.len());
            result.push_str("{ \n");
            for statement in statements {
                result.push_str("    ");
                result.push_str(statement.location.text().trim());
                result.push('\n');
            }
            result.push('}');
            result
        });
        writeln!(out, "{name} () \n{rendered}")?;
        if flags.len() > 1 {
            writeln!(out, "declare -{flags} {name}")?;
        }
        Ok(())
    }
}
