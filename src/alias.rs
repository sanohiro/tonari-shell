use std::collections::BTreeMap;
use std::io::{self, Write};
use std::rc::Rc;

use crate::Session;

#[derive(Clone, Debug, Default)]
pub struct Aliases(pub Option<Rc<BTreeMap<String, String>>>);

impl Aliases {
    pub(crate) fn insert(&mut self, name: &str, value: &str) -> Result<(), &'static str> {
        if name.is_empty()
            || name
                .chars()
                .any(|c| c.is_whitespace() || "/$`=;&|()<>'\"\\".contains(c))
        {
            return Err("invalid alias name");
        }
        Rc::make_mut(self.0.get_or_insert_with(Rc::default)).insert(name.into(), value.into());
        Ok(())
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.0.iter().flat_map(|map| map.keys())
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.as_ref()?.get(name).map(String::as_str)
    }
}

pub fn declaration(name: &str, value: &str) -> String {
    format!(
        "{}{name}='{}'",
        prefix(name, true),
        value.replace('\'', "'\\''")
    )
}

fn prefix(name: &str, command: bool) -> &'static str {
    if !command {
        ""
    } else if name.starts_with('-') {
        "alias -- "
    } else {
        "alias "
    }
}

pub fn builtin(cmd: &str, mut args: &[String], session: &mut Session) -> u8 {
    let remove = cmd == "unalias";
    let mut print_all = false;
    let mut clear = false;
    while let Some((arg, rest)) = args.split_first() {
        if arg == "--" {
            args = rest;
            break;
        }
        let Some(flags) = arg.strip_prefix('-').filter(|s| !s.is_empty()) else {
            break;
        };
        for flag in flags.chars() {
            match (remove, flag) {
                (false, 'p') => print_all = true,
                (true, 'a') => clear = true,
                _ => {
                    eprintln!("tonari: {cmd}: -{flag}: invalid option");
                    return 2;
                }
            }
        }
        args = rest;
    }
    if clear {
        session.aliases.0 = None;
        return 0;
    }
    if remove && args.is_empty() {
        return 2;
    }
    let mut out = io::stdout().lock();
    let command_prefix = !session.posix || print_all;
    if !remove && (print_all || args.is_empty()) {
        if let Some(map) = &session.aliases.0 {
            for (name, value) in map.iter() {
                let prefix = prefix(name, command_prefix);
                if writeln!(out, "{prefix}{name}='{}'", value.replace('\'', "'\\''")).is_err() {
                    return 1;
                }
            }
        }
        return u8::from(out.flush().is_err());
    }
    let mut status = 0;
    for arg in args {
        if !remove {
            if let Some((name, value)) = arg.split_once('=') {
                if let Err(e) = session.aliases.insert(name, value) {
                    eprintln!("tonari: alias: {name}: {e}");
                    status = 1;
                }
                continue;
            }
        }
        if remove
            && session
                .aliases
                .0
                .as_mut()
                .is_some_and(|map| Rc::make_mut(map).remove(arg).is_some())
        {
            continue;
        }
        if let Some(value) = session.aliases.get(arg) {
            let prefix = prefix(arg, command_prefix);
            if writeln!(out, "{prefix}{arg}='{}'", value.replace('\'', "'\\''")).is_err() {
                return 1;
            }
        } else {
            eprintln!("tonari: {cmd}: {arg}: not found");
            status = 1;
        }
    }
    if out.flush().is_err() { 1 } else { status }
}
