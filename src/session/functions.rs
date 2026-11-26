use super::{Function, Session};
use crate::parser::{Command, FunctionDefinition, is_function_name, parse_final};
use std::{
    ffi::CStr,
    io::{self, Write},
    rc::Rc,
};

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
        self.get("FUNCNEST")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .is_some_and(|max| max > 0 && self.function_depth >= max)
            || self.function_depth >= 128 && self.deep_function_nesting_full()
    }

    #[cold]
    fn deep_function_nesting_full(&self) -> bool {
        // The evaluator runs on the process main stack. For calls beyond the
        // original guard, keep at least half its soft limit for the environment,
        // signal frames and nested evaluation. Honor changes made by ulimit.
        let Some(origin) = crate::MAIN_STACK_ORIGIN.get() else {
            return true;
        };
        if self.function_depth >= 256 {
            return true;
        }
        let mut limit = std::mem::MaybeUninit::uninit();
        if unsafe { libc::getrlimit(libc::RLIMIT_STACK, limit.as_mut_ptr()) } != 0 {
            return true;
        }
        let used = origin.abs_diff(std::ptr::from_ref(&limit).addr());
        let budget = unsafe { limit.assume_init() }.rlim_cur / 2;
        used as u64 + 256 * 1024 >= budget
    }

    pub(crate) fn function_source(&self) -> &str {
        self.source_names
            .last()
            .map(AsRef::as_ref)
            .unwrap_or_else(|| {
                if self.command_string || self.stdin_script {
                    "main"
                } else {
                    &self.arg0
                }
            })
    }

    pub(crate) fn define_function(&mut self, name: String, definition: Rc<FunctionDefinition>) {
        let source = self
            .source_names
            .last()
            .cloned()
            .unwrap_or_else(|| self.function_source().into());
        let function = Function { definition, source };
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
                if name.contains('/') {
                    eprintln!("tonari: error importing function definition for `{name}'");
                    continue;
                }
                let key = format!("BASH_FUNC_{name}%%");
                self.function_environment.insert(key.clone(), None);
                // Parse only the declaration body supplied by the protocol.
                // The environment key stays a name throughout the import.
                let Ok(mut parsed) = parse_final(&format!("__tonari_import {value}")) else {
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
        let definition = &self.functions[name].definition;
        let body = definition.location.text();
        let statements = definition.body.statements();
        if body.starts_with('{') && body.ends_with('}') && statements.iter().all(|s|
            s.condition == crate::parser::Condition::Always && !s.negated
            && matches!(&s.command, Command::Pipeline(stages) if stages.iter().all(|c| c.compound.is_none() && c.redirects.is_empty()))
            && !s.location.text().contains('\n')) {
            writeln!(out, "{name} () \n{{ ")?;
            for s in statements { writeln!(out, "    {}", s.location.text().trim())?; }
            writeln!(out, "}}")?;
        } else {
            writeln!(out, "{name} () \n{body}")?;
        }
        let flags = self.function_attributes(name);
        if flags.len() > 1 {
            writeln!(out, "declare -{flags} {name}")?;
        }
        Ok(())
    }
}
