use std::collections::BTreeMap;
use std::io::{self, Write};
use std::rc::Rc;

use crate::Session;

#[derive(Clone, Default)]
pub struct Aliases(pub Option<Rc<BTreeMap<String, String>>>);

impl Aliases {
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.0.iter().flat_map(|map| map.keys())
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.as_ref()?.get(name).map(String::as_str)
    }
}

pub fn declaration(name: &str, value: &str) -> String {
    format!("alias {name}='{}'", value.replace('\'', "'\\''"))
}

pub fn builtin(cmd: &str, mut args: &[String], session: &mut Session) -> u8 {
    let remove = cmd == "unalias";
    let print_all = !remove && args.first().is_some_and(|s| s == "-p");
    if print_all {
        args = &args[1..];
    }
    if remove && args.first().is_some_and(|s| s == "-a") {
        session.aliases.0 = None;
        return 0;
    }
    if args.first().is_some_and(|s| s == "--") {
        args = &args[1..];
    }
    if remove && args.is_empty() {
        return 2;
    }
    let mut out = io::stdout().lock();
    let prefix = if session.posix && !print_all {
        ""
    } else {
        "alias "
    };
    if (print_all || args.is_empty()) && !remove {
        if let Some(map) = &session.aliases.0 {
            for (name, value) in map.iter() {
                if writeln!(out, "{prefix}{name}='{}'", value.replace('\'', "'\\''")).is_err() {
                    return 1;
                }
            }
        }
    }
    let mut status = 0;
    for arg in args {
        if !remove {
            if let Some((name, value)) = arg.split_once('=') {
                if name.is_empty()
                    || name
                        .chars()
                        .any(|c| c.is_whitespace() || "/$`=;&|()<>'\"\\".contains(c))
                {
                    eprintln!("tonari: alias: {name}: invalid alias name");
                    status = 1;
                } else {
                    Rc::make_mut(session.aliases.0.get_or_insert_with(Rc::default))
                        .insert(name.into(), value.into());
                }
                continue;
            }
        }
        if let Some(value) = session.aliases.get(arg) {
            if remove {
                Rc::make_mut(session.aliases.0.as_mut().unwrap()).remove(arg);
            } else if writeln!(out, "{prefix}{arg}='{}'", value.replace('\'', "'\\''")).is_err() {
                return 1;
            }
        } else {
            eprintln!("tonari: {cmd}: {arg}: not found");
            status = 1;
        }
    }
    if out.flush().is_err() { 1 } else { status }
}
