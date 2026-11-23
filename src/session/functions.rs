use super::{Function, Session};
use crate::parser::{Command, Location, Statement, is_function_name, parse_final};
use std::{
    ffi::CStr,
    io::{self, Write},
    rc::Rc,
};

impl Function {
    fn export_value(&self) -> String {
        let body = self.definition.text();
        if body.starts_with('{') && body.ends_with('}') {
            format!("() {body}")
        } else {
            format!("() {{\n{body}\n}}")
        }
    }
}

impl Session {
    pub(crate) fn define_function(
        &mut self,
        name: String,
        body: Vec<Statement>,
        definition: Location,
    ) {
        let source = self
            .source_names
            .last()
            .cloned()
            .unwrap_or_else(|| "environment".into());
        let function = Rc::new(Function {
            body,
            definition,
            source,
        });
        if self.allexport || self.function_exported(&name) {
            self.function_environment
                .insert(format!("BASH_FUNC_{name}%%"), Some(function.export_value()));
        }
        self.functions.insert(name, function);
    }

    pub fn import_functions(&mut self) {
        // Startup is single-threaded. Leave the ordinary inherited environment in place.
        let mut env = crate::execute::inherited_env();
        unsafe {
            while !(*env).is_null() {
                let raw = *env;
                env = env.add(1);
                if *raw != b'B' as libc::c_char
                    || libc::strncmp(raw, c"BASH_FUNC_".as_ptr(), 10) != 0
                {
                    continue;
                }
                if libc::getuid() != libc::geteuid() || libc::getgid() != libc::getegid() {
                    return;
                }
                let entry = CStr::from_ptr(raw).to_bytes();
                let Some(rest) = entry.strip_prefix(b"BASH_FUNC_") else {
                    continue;
                };
                let Ok(rest) = std::str::from_utf8(rest) else {
                    continue;
                };
                let Some((name, value)) = rest.split_once("%%=") else {
                    continue;
                };
                if !is_function_name(name) || !value.starts_with("() {") {
                    continue;
                }
                let key = format!("BASH_FUNC_{name}%%");
                self.function_environment.insert(key.clone(), None);
                let Ok(mut parsed) = parse_final(&format!("{name} {value}")) else {
                    continue;
                };
                if parsed.len() != 1 {
                    continue;
                }
                let statement = parsed.pop().unwrap();
                if statement.negated {
                    continue;
                }
                if let Command::Function {
                    name: actual,
                    body,
                    definition,
                } = statement.command
                    && actual == name
                {
                    self.functions.insert(
                        actual,
                        Rc::new(Function {
                            body,
                            definition,
                            source: "environment".into(),
                        }),
                    );
                    self.function_environment
                        .insert(key, Some(value.to_owned()));
                }
            }
        }
    }

    pub fn export_function(&mut self, name: &str, on: bool) -> Result<(), &'static str> {
        let function = self.functions.get(name).ok_or("not a function")?;
        let value = on.then(|| function.export_value());
        self.function_environment
            .insert(format!("BASH_FUNC_{name}%%"), value);
        Ok(())
    }

    pub fn function_exported(&self, name: &str) -> bool {
        !self.function_environment.is_empty()
            && self
                .function_environment
                .get(&format!("BASH_FUNC_{name}%%"))
                .is_some_and(Option::is_some)
    }

    pub fn forget_function(&mut self, name: &str) {
        if self.function_exported(name) {
            let _ = self.export_function(name, false);
        }
        self.traced_functions.remove(name);
        self.functions.remove(name);
    }

    pub fn function_attributes(&self, name: &str) -> String {
        let mut flags = String::from("f");
        if self.readonly_functions.contains(name) {
            flags.push('r');
        }
        if self.traced_functions.contains(name) {
            flags.push('t');
        }
        if self.function_exported(name) {
            flags.push('x');
        }
        flags
    }

    pub fn print_function(&self, out: &mut impl Write, name: &str) -> io::Result<()> {
        let body = self.functions[name].definition.text();
        writeln!(out, "{name} () \n{body}")?;
        let flags = self.function_attributes(name);
        if flags.len() > 1 {
            writeln!(out, "declare -{flags} {name}")?;
        }
        Ok(())
    }
}
