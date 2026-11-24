use crate::{Session, execute::builtin_names};
use std::io::{self, Write};

const SPECIAL: &[&str] = &[
    ":", ".", "break", "continue", "eval", "exec", "exit", "export", "readonly", "return", "set",
    "shift", "times", "trap", "unset",
];

pub fn caller(session: &Session, args: &[String]) -> u8 {
    let args = if args.first().is_some_and(|a| a == "--") {
        &args[1..]
    } else {
        args
    };
    let Some(first) = session.call_lines.last() else {
        return 1;
    };
    let mut sources = session.source_names.iter().rev().skip(1);
    let mut out = io::stdout().lock();
    let line = if let Some(arg) = args.first() {
        let Some(depth) = arg.parse::<usize>().ok() else {
            return 1;
        };
        let Some(((name, source), line)) = session
            .function_names
            .iter()
            .rev()
            .skip(1)
            .nth(depth)
            .zip(sources.nth(depth))
            .zip(session.call_lines.iter().rev().nth(depth))
        else {
            return 1;
        };
        writeln!(out, "{line} {name} {source}")
    } else {
        writeln!(
            out,
            "{first} {}",
            sources.next().map_or("NULL", AsRef::as_ref)
        )
    };
    u8::from(line.is_err())
}

pub fn enable(session: &mut Session, mut args: &[String]) -> u8 {
    let (mut disable, mut list, mut all, mut special) = (false, false, false, false);
    while let Some((opt, rest)) = args.split_first() {
        if opt == "--" {
            args = rest;
            break;
        }
        if !opt.starts_with('-') || opt == "-" {
            break;
        }
        for c in opt[1..].chars() {
            match c {
                'n' => disable = true,
                'p' => list = true,
                'a' => all = true,
                's' => special = true,
                _ => {
                    eprintln!("tonari: enable: unsupported option -{c}");
                    return 2;
                }
            }
        }
        args = rest;
    }
    if list || args.is_empty() {
        let mut names = builtin_names().to_vec();
        names.sort_unstable();
        let mut out = io::stdout().lock();
        for name in names {
            let off = session.disabled_builtins.contains(name);
            if (!special || SPECIAL.contains(&name))
                && (all || disable == off)
                && writeln!(out, "enable {}{name}", if off { "-n " } else { "" }).is_err()
            {
                return 1;
            }
        }
        return 0;
    }
    let mut status = 0;
    for name in args {
        if !builtin_names().contains(&name.as_str()) {
            eprintln!("tonari: enable: {name}: not a shell builtin");
            status = 1;
        } else if disable {
            session.disabled_builtins.insert(name.clone());
        } else {
            session.disabled_builtins.remove(name);
        }
    }
    status
}

pub fn suspend(session: &Session, mut args: &[String]) -> u8 {
    let force = args.first().is_some_and(|s| s == "-f");
    if force {
        args = &args[1..];
    }
    if args.first().is_some_and(|s| s == "--") {
        args = &args[1..];
    }
    if !args.is_empty() {
        eprintln!("tonari: suspend: expected suspend [-f]");
        return 2;
    }
    if !force && (session.login || !session.jobs.monitor) {
        eprintln!("tonari: suspend: cannot suspend this shell");
        return 1;
    }
    // A nested shell can share a process group with its launcher when job control is off.
    // Signal its own group only when it owns that group.
    let pid = unsafe { libc::getpid() };
    let target = if unsafe { libc::getpgrp() } == pid {
        -pid
    } else {
        pid
    };
    u8::from(unsafe { libc::kill(target, libc::SIGSTOP) } != 0)
}

pub fn help(mut args: &[String]) -> u8 {
    let mut mode = ' ';
    while let Some((opt, rest)) = args.split_first() {
        if opt == "--" {
            args = rest;
            break;
        }
        if !opt.starts_with('-') || opt == "-" {
            break;
        }
        for c in opt[1..].chars() {
            if !"dms".contains(c) {
                return 2;
            }
            mode = c;
        }
        args = rest;
    }
    let mut out = io::stdout().lock();
    let topics = include_str!("help.txt");
    if args.is_empty() {
        for topic in topics.lines() {
            if writeln!(out, "{}", topic.split('|').next().unwrap()).is_err() {
                return 1;
            }
        }
        return 0;
    }
    let mut any = false;
    for pattern in args {
        let matches = |name: &str| crate::pattern::extended::matches(name, pattern);
        let exact = topics
            .lines()
            .any(|s| matches(s.split(' ').next().unwrap().split('|').next().unwrap()));
        for row in topics.lines() {
            let (syntax, description) = row.split_once('|').unwrap();
            let name = syntax.split(' ').next().unwrap();
            if !(matches(name) || !exact && name.starts_with(pattern)) {
                continue;
            }
            any = true;
            let result = match mode {
                's' => writeln!(out, "{name}: {syntax}"),
                'd' => writeln!(out, "{name} - {description}"),
                'm' => writeln!(
                    out,
                    "NAME\n    {name} - {description}\n\nSYNOPSIS\n    {syntax}\n\nDESCRIPTION\n    {description}"
                ),
                _ => writeln!(out, "{name}: {syntax}\n    {description}"),
            };
            if result.is_err() {
                return 1;
            }
        }
    }
    if !any {
        eprintln!("tonari: help: no matching help topic");
    }
    u8::from(!any)
}
