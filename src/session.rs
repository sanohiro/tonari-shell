pub(crate) mod array;
mod declare;
mod options;
pub(crate) mod printf;
mod read;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use crate::parser::{Statement, is_identifier};

#[derive(Clone)]
struct Variable {
    array_assigned: bool,
    elements: Option<Rc<BTreeMap<i64, String>>>,
    declared: bool,
    value: Option<String>,
    exported: bool,
}

#[derive(Clone)]
struct ShellPid(u32);

impl Default for ShellPid {
    fn default() -> Self {
        Self(std::process::id())
    }
}

#[derive(Clone, Default)]
pub struct Session {
    // Only changed variables are stored here. Simple commands inherit the
    // original process environment without building a copy on startup.
    variables: BTreeMap<String, Variable>,
    logical_cwd: Option<PathBuf>,
    shell_pid: ShellPid,
    pub interactive: bool,
    pub command_string: bool,
    pub tested_depth: u32,
    pub errexit: bool,
    pub nounset: bool,
    pub pipefail: bool,
    pub noglob: bool,
    pub shopt: options::ShellOptions,
    pub(crate) external_env: std::cell::RefCell<Option<Rc<crate::execute::Environment>>>,
    pub regex: Option<Rc<crate::test_expression::Compiled>>,
    pub substitutions: crate::execute::process_substitution::Substitutions,
    pub jobs: crate::jobs::Jobs,
    pub hashes: crate::execute::hash::Table,
    pub last_background: Option<libc::pid_t>,
    pub arg0: String,
    pub positional: Vec<String>,
    pub source_depth: u32,
    pub function_depth: u32,
    pub function_names: Vec<String>,
    pub source_names: Vec<String>,
    pub function_sources: BTreeMap<String, String>,
    pub loop_depth: u32,
    pub aliases: crate::alias::Aliases,
    pub completions: crate::completion_spec::Registry,
    pub functions: BTreeMap<String, Rc<Vec<Statement>>>,
    locals: Vec<Bindings>,
}

#[derive(Clone, Default)]
pub struct Bindings(BTreeMap<String, Option<Variable>>);

impl Session {
    pub fn variable_names(&self) -> Vec<String> {
        let mut names: std::collections::BTreeSet<_> = std::env::vars_os()
            .filter_map(|(k, _)| k.into_string().ok())
            .filter(|k| is_identifier(k))
            .collect();
        names.extend(["PWD", "IFS", "PATH"].into_iter().map(str::to_owned));
        for (key, var) in &self.variables {
            if var.declared {
                names.insert(key.clone());
            } else {
                names.remove(key);
            }
        }
        names.into_iter().collect()
    }

    pub fn shell_pid(&self) -> u32 {
        self.shell_pid.0
    }

    pub fn option_flags(&self) -> String {
        let mut flags = String::new();
        if self.errexit {
            flags.push('e');
        }
        if self.noglob {
            flags.push('f');
        }
        if self.hashes.enabled {
            flags.push('h');
        }
        if self.interactive {
            flags.push('i');
        }
        if self.nounset {
            flags.push('u');
        }
        flags
    }

    pub fn printf(&mut self, args: &[String]) -> u8 {
        printf::run(self, args, None)
    }

    pub fn read(&mut self, args: &[String]) -> u8 {
        read::run(self, args)
    }

    pub fn shift(&mut self, args: &[String]) -> u8 {
        let args = match args.split_first() {
            Some((flag, rest)) if flag == "--" => rest,
            _ => args,
        };
        if args.len() > 1 {
            eprintln!("tonari: shift: too many arguments");
            return 1;
        }
        let count = args.first().map_or(Ok(1), |s| s.trim().parse::<i64>());
        let Ok(count) = count else {
            eprintln!("tonari: shift: numeric argument required");
            return 1;
        };
        if count < 0 || count as u64 > self.positional.len() as u64 {
            return 1;
        }
        self.positional.drain(..count as usize);
        0
    }

    pub fn set(&mut self, args: &[String]) -> u8 {
        if args.is_empty() {
            let mut values: BTreeMap<_, _> = std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
                .collect();
            for (name, var) in &self.variables {
                if let Some(value) = &var.value {
                    values.insert(name.clone(), value.clone());
                } else {
                    values.remove(name);
                }
            }
            let mut out = io::stdout().lock();
            for (name, value) in values {
                if is_identifier(&name)
                    && writeln!(out, "{name}='{}'", value.replace('\'', "'\\''")).is_err()
                {
                    return 1;
                }
            }
            return out.flush().map_or(1, |()| 0);
        }
        let mut i = 0;
        let mut replace_positionals = false;
        while let Some(opt) = args.get(i) {
            if opt == "--" {
                i += 1;
                replace_positionals = true;
                break;
            }
            if opt == "-" {
                i += 1;
                break;
            }
            let Some(prefix) = opt.as_bytes().first() else {
                break;
            };
            if !matches!(prefix, b'-' | b'+') || opt.len() < 2 {
                break;
            }
            let enable = *prefix == b'-';
            i += 1;
            for flag in opt[1..].chars() {
                match flag {
                    'e' => self.errexit = enable,
                    'f' => self.noglob = enable,
                    'h' => self.hashes.enabled = enable,
                    'u' => self.nounset = enable,
                    'o' => {
                        let Some(name) = args.get(i) else {
                            let mut out = io::stdout().lock();
                            for (name, on) in [
                                ("errexit", self.errexit),
                                ("hashall", self.hashes.enabled),
                                ("noglob", self.noglob),
                                ("nounset", self.nounset),
                                ("pipefail", self.pipefail),
                            ] {
                                let result = if enable {
                                    writeln!(out, "{name:16}{}", if on { "on" } else { "off" })
                                } else {
                                    writeln!(out, "set {}o {name}", if on { "-" } else { "+" })
                                };
                                if result.is_err() {
                                    return 1;
                                }
                            }
                            return out.flush().map_or(1, |()| 0);
                        };
                        i += 1;
                        if let Some(value) = self.option_value(name, true) {
                            *value = enable;
                        } else {
                            eprintln!("tonari: set: {name}: unsupported option");
                            return 2;
                        }
                    }
                    _ => {
                        eprintln!("tonari: set: {opt}: unsupported option");
                        return 2;
                    }
                }
            }
        }
        if i < args.len() || replace_positionals {
            self.positional = args[i..].to_vec();
        }
        0
    }

    pub fn get(&self, name: &str) -> Option<String> {
        if let Some(base) = name.strip_prefix('\u{1}') {
            return Some(
                self.indices(base)
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if let Some((base, index)) = array::subscript(name) {
            let i = self.resolve_index(base, index).ok()?;
            return self.element(base, i);
        }
        if let Some(var) = self.variables.get(name) {
            return var
                .elements
                .as_ref()
                .map_or_else(|| var.value.clone(), |a| a.get(&0).cloned());
        }
        if matches!(name, "BASH_SOURCE" | "FUNCNAME") {
            return self.element(name, 0);
        }
        self.inherited_value(name)
    }

    fn inherited_value(&self, name: &str) -> Option<String> {
        match name {
            "PWD" => self
                .logical_directory()
                .ok()?
                .into_os_string()
                .into_string()
                .ok(),
            "BASH" => std::env::current_exe()
                .ok()?
                .into_os_string()
                .into_string()
                .ok(),
            "IFS" => Some(" \t\n".into()),
            "PATH" => Some(std::env::var(name).unwrap_or_else(|_| {
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into()
            })),
            _ => std::env::var(name).ok(),
        }
    }

    pub fn assign(&mut self, name: &str, value: &str) {
        if name == "PATH" {
            self.hashes.clear();
        }
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(elements) = var.elements.as_mut().map(Rc::make_mut) {
                elements.insert(0, value.into());
                var.array_assigned = true;
                return;
            }
        }
        let exported = self.is_exported(name);
        self.variables.insert(
            name.into(),
            Variable {
                array_assigned: false,
                elements: None,
                declared: true,
                value: Some(value.into()),
                exported,
            },
        );
    }

    fn is_exported(&self, name: &str) -> bool {
        self.variables.get(name).map_or_else(
            || matches!(name, "PWD" | "OLDPWD") || std::env::var_os(name).is_some(),
            |var| var.exported,
        )
    }

    pub fn save_variables(&self, names: &[&str]) -> Bindings {
        Bindings(
            names
                .iter()
                .map(|&name| (name.into(), self.variables.get(name).cloned()))
                .collect(),
        )
    }

    pub fn bind(&mut self, assignments: &[(String, String)]) -> Bindings {
        let mut saved = Bindings::default();
        for (name, value) in assignments {
            if name == "PATH" {
                self.hashes.clear();
            }
            saved
                .0
                .entry(name.clone())
                .or_insert_with(|| self.variables.get(name).cloned());
            self.variables.insert(
                name.clone(),
                Variable {
                    array_assigned: false,
                    elements: None,
                    declared: true,
                    value: Some(value.clone()),
                    exported: true,
                },
            );
        }
        saved
    }

    pub fn restore(&mut self, bindings: Bindings) {
        for (name, original) in bindings.0 {
            if name == "PATH" {
                self.hashes.clear();
            }
            if let Some(var) = original {
                self.variables.insert(name, var);
            } else {
                self.variables.remove(&name);
            }
        }
    }

    // Preserve arbitrary inherited environment bytes. Only shell-authored
    // changes and command-prefix assignments use the UTF-8 language model.
    pub fn environment_changes<'a>(
        &'a self,
        assignments: &'a [(String, String)],
    ) -> impl Iterator<Item = (&'a str, Option<&'a str>)> {
        self.variables
            .iter()
            .filter_map(|(name, var)| {
                if var.exported && var.elements.is_none() && var.value.is_some() {
                    Some((name.as_str(), var.value.as_deref()))
                } else if std::env::var_os(name).is_some() {
                    Some((name.as_str(), None))
                } else {
                    None
                }
            })
            .chain(
                assignments
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(v.as_str()))),
            )
    }

    pub fn search_path(&self, assignments: &[(String, String)]) -> OsString {
        if let Some((_, path)) = assignments.iter().rev().find(|(k, _)| k == "PATH") {
            return path.into();
        }
        if let Some(var) = self.variables.get("PATH") {
            return var.value.as_deref().unwrap_or("").into();
        }
        std::env::var_os("PATH").unwrap_or_else(|| self.get("PATH").unwrap().into())
    }

    pub fn overrides_search_path(&self, assignments: &[(String, String)]) -> bool {
        self.variables.contains_key("PATH") || assignments.iter().any(|(k, _)| k == "PATH")
    }

    pub fn enter_function(&mut self) {
        self.locals.push(Bindings::default());
        self.function_depth += 1;
    }

    pub fn leave_function(&mut self) {
        let locals = self.locals.pop().expect("function scope");
        self.restore(locals);
        self.function_depth -= 1;
    }

    pub fn export(&mut self, args: &[String], bindings: &mut Bindings) -> u8 {
        let mut remove = false;
        let mut i = 0;
        while let Some(opt) = args.get(i) {
            if opt == "--" {
                i += 1;
                break;
            }
            if !opt.starts_with('-') || opt == "-" {
                break;
            }
            for flag in opt[1..].chars() {
                match flag {
                    'n' => remove = true,
                    'p' => {}
                    _ => {
                        eprintln!("tonari: export: unsupported option {opt}");
                        return 2;
                    }
                }
            }
            i += 1;
        }
        if i == args.len() {
            return self.list_exported().map_or(1, |()| 0);
        }
        let mut status = 0;
        for arg in &args[i..] {
            let (key, value) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
            let (name, append) = if value.is_some() {
                key.strip_suffix('+').map_or((key, false), |k| (k, true))
            } else {
                (key, false)
            };
            if !is_identifier(name) {
                eprintln!("tonari: export: {arg}: invalid identifier");
                status = 1;
                continue;
            }
            if name == "PATH" && value.is_some() {
                self.hashes.clear();
            }
            if let Some(var) = self.variables.get_mut(name) {
                if let Some(array) = var.elements.as_mut().map(Rc::make_mut) {
                    if let Some(v) = value {
                        if append {
                            array.entry(0).or_default().push_str(v);
                        } else {
                            array.insert(0, v.into());
                        }
                        var.array_assigned = true;
                    }
                    var.exported = !remove;
                    if !remove {
                        bindings.0.remove(name);
                    }
                    continue;
                }
            }
            let value = match value {
                Some(v) if append => Some(self.get(name).unwrap_or_default() + v),
                Some(v) => Some(v.to_owned()),
                None => self.get(name),
            };
            self.variables.insert(
                name.into(),
                Variable {
                    array_assigned: false,
                    elements: None,
                    declared: true,
                    value,
                    exported: !remove,
                },
            );
            if !remove {
                // An explicit export inside this invocation promotes its prefix
                // binding into the surrounding shell.
                bindings.0.remove(name);
            }
        }
        status
    }

    fn list_exported(&self) -> io::Result<()> {
        let mut vars: BTreeMap<String, Option<String>> = std::env::vars_os()
            .filter_map(|(k, v)| {
                let name = k.into_string().ok()?;
                is_identifier(&name).then(|| (name, v.into_string().ok()))
            })
            .collect();
        vars.insert("PWD".into(), self.get("PWD"));
        vars.entry("OLDPWD".into()).or_insert(None);
        vars.remove("_");
        for (name, var) in &self.variables {
            if var.exported {
                vars.insert(name.clone(), var.value.clone());
            } else {
                vars.remove(name);
            }
        }
        let mut out = io::stdout().lock();
        for (name, value) in vars {
            if let Some(var) = self.variables.get(&name) {
                print_declaration(&mut out, &name, var)?;
                continue;
            }
            print_declaration(
                &mut out,
                &name,
                &Variable {
                    array_assigned: false,
                    elements: None,
                    declared: true,
                    value,
                    exported: true,
                },
            )?;
        }
        out.flush()
    }

    pub fn unset(&mut self, args: &[String]) -> u8 {
        let mut index = 0;
        let mut functions_only = false;
        let mut variables_only = false;
        while let Some(opt) = args.get(index) {
            if opt == "--" {
                index += 1;
                break;
            }
            if !opt.starts_with('-') || opt == "-" {
                break;
            }
            for flag in opt[1..].chars() {
                match flag {
                    'v' => variables_only = true,
                    'f' => functions_only = true,
                    _ => {
                        eprintln!("tonari: unset: unsupported option {opt}");
                        return 2;
                    }
                }
            }
            index += 1;
        }
        if functions_only && variables_only {
            eprintln!("tonari: unset: cannot combine -f and -v");
            return 1;
        }
        for name in &args[index..] {
            if !functions_only {
                if let Some((base, index)) = array::subscript(name) {
                    match self.resolve_index(base, index) {
                        Ok(i) => self.unset_element(base, i),
                        Err(e) => {
                            eprintln!("tonari: unset: {e}");
                            return 1;
                        }
                    }
                    continue;
                }
            }
            if functions_only {
                self.functions.remove(name);
                continue;
            }
            if is_identifier(name) {
                if name == "PATH" {
                    self.hashes.clear();
                }
                if !variables_only && self.get(name).is_none() {
                    self.functions.remove(name);
                }
                self.variables.insert(
                    name.clone(),
                    Variable {
                        array_assigned: false,
                        elements: None,
                        declared: self
                            .locals
                            .last()
                            .is_some_and(|scope| scope.0.contains_key(name)),
                        value: None,
                        exported: false,
                    },
                );
            }
        }
        0
    }

    pub fn logical_directory(&self) -> io::Result<PathBuf> {
        if let Some(cwd) = &self.logical_cwd {
            return Ok(cwd.clone());
        }
        if let Some(pwd) = std::env::var_os("PWD") {
            let path = Path::new(&pwd);
            if path.is_absolute()
                && !path.components().any(|c| c == Component::ParentDir)
                && same_directory(path, Path::new("."))
            {
                return Ok(path.into());
            }
        }
        std::env::current_dir()
    }

    pub fn directory_builtin(&mut self, name: &str, args: &[String]) -> u8 {
        let mut physical = false;
        let mut i = 0;
        while let Some(opt) = args.get(i) {
            if opt == "--" {
                i += 1;
                break;
            }
            if !opt.starts_with('-') || opt == "-" {
                break;
            }
            for flag in opt[1..].chars() {
                match flag {
                    'L' => physical = false,
                    'P' => physical = true,
                    _ => {
                        eprintln!("tonari: {name}: unsupported option {opt}");
                        return 2;
                    }
                }
            }
            i += 1;
        }
        let result = if name == "pwd" {
            (if physical {
                std::env::current_dir()
            } else {
                self.logical_directory()
            })
            .and_then(|p| print_path(&p))
        } else {
            if args.len() - i > 1 {
                eprintln!("tonari: cd: too many arguments");
                return 1;
            }
            let arg = args.get(i).map(String::as_str);
            let (target, previous) = match arg {
                None => (self.get("HOME"), false),
                Some("-") => (self.get("OLDPWD"), true),
                Some(s) => (Some(s.into()), false),
            };
            let Some(target) = target else {
                eprintln!(
                    "tonari: cd: {} not set",
                    if previous { "OLDPWD" } else { "HOME" }
                );
                return 1;
            };
            self.change_directory(Path::new(&target), physical, previous, arg.is_some())
        };
        result.map_or_else(
            |e| {
                eprintln!("tonari: {name}: {e}");
                1
            },
            |()| 0,
        )
    }

    fn change_directory(
        &mut self,
        target: &Path,
        physical: bool,
        previous: bool,
        explicit: bool,
    ) -> io::Result<()> {
        let old_pwd = self.get("PWD");
        let base = self.logical_directory();
        let mut paths = Vec::new();
        if explicit && !previous && matches!(target.components().next(), Some(Component::Normal(_)))
        {
            if let Some(cdpath) = self.get("CDPATH") {
                paths.extend(std::env::split_paths(&cdpath).map(|dir| {
                    let print = !dir.as_os_str().is_empty();
                    (dir.join(target), print)
                }));
            }
        }
        paths.push((target.into(), previous));
        let mut error = io::Error::from(io::ErrorKind::NotFound);
        for (candidate, print) in paths {
            if candidate.as_os_str().is_empty() {
                continue;
            }
            let logical = if physical {
                None
            } else {
                let joined = if candidate.is_absolute() {
                    Some(candidate.clone())
                } else {
                    base.as_ref().ok().map(|base| base.join(&candidate))
                };
                match joined.as_deref().map(normalize_directory).transpose() {
                    Ok(p) => p,
                    Err(e) => {
                        error = e;
                        continue;
                    }
                }
            };
            let destination = logical.as_deref().unwrap_or(&candidate);
            if let Err(e) = std::env::set_current_dir(destination) {
                error = e;
                continue;
            }
            self.hashes.directory_changed();
            self.logical_cwd = logical;
            if self.logical_cwd.is_none() {
                self.logical_cwd = Some(std::env::current_dir()?);
            }
            let cwd = self.logical_cwd.as_ref().unwrap().clone();
            let old_export = self.is_exported("OLDPWD");
            self.variables.insert(
                "OLDPWD".into(),
                Variable {
                    array_assigned: false,
                    elements: None,
                    declared: true,
                    value: old_pwd,
                    exported: old_export,
                },
            );
            self.assign(
                "PWD",
                cwd.to_str().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 directory")
                })?,
            );
            if print {
                print_path(&cwd)?;
            }
            return Ok(());
        }
        Err(io::Error::new(
            error.kind(),
            format!("{}: {error}", target.display()),
        ))
    }
}

fn print_declaration(out: &mut impl Write, name: &str, var: &Variable) -> io::Result<()> {
    let flags = match (var.elements.is_some(), var.exported) {
        (true, true) => "-ax",
        (true, false) => "-a",
        (false, true) => "-x",
        (false, false) => "--",
    };
    write!(out, "declare {flags} {name}")?;
    if let Some(array) = &var.elements {
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

fn print_value(out: &mut impl Write, value: &str) -> io::Result<()> {
    if value.bytes().any(|c| c.is_ascii_control()) {
        return out.write_all(&printf::ansi_quote(value.as_bytes(), false));
    }
    write!(out, "\"")?;
    for c in value.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            write!(out, "\\")?;
        }
        write!(out, "{c}")?;
    }
    write!(out, "\"")
}

fn same_directory(left: &Path, right: &Path) -> bool {
    match (std::fs::metadata(left), std::fs::metadata(right)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

fn normalize_directory(path: &Path) -> io::Result<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir {
            if !std::fs::metadata(&result)?.is_dir() {
                return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
            }
            result.pop();
        } else {
            result.push(component);
        }
    }
    Ok(result)
}

fn print_path(path: &Path) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(path.as_os_str().as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_locals_and_command_prefixes_restore_nested_shell_values() {
        let key = "TONARI_TEST_SCOPE";
        let mut session = Session::default();
        session.assign(key, "global");
        session.enter_function();
        assert_eq!(session.declare("local", &[format!("{key}=local")]), 0);
        let prefix = session.bind(&[(key.into(), "temporary".into())]);
        assert_eq!(session.get(key).as_deref(), Some("temporary"));
        assert!(session.is_exported(key));
        session.restore(prefix);
        assert_eq!(session.get(key).as_deref(), Some("local"));
        session.leave_function();
        assert_eq!(session.get(key).as_deref(), Some("global"));
    }
}
