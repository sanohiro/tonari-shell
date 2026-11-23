use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::Session;

macro_rules! actions {
    ($($variant:ident => $name:literal [$($key:literal),*]),+ $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(super) enum Action { $($variant),+ }
        const ACTIONS: &[(Action, &str, &[&[u8]])] = &[$((Action::$variant, $name, &[$($key),*])),+];
    }
}
actions! {
    Abort => "abort" [b"\x07", b"\x18\x07", b"\x1b\x07"],
    Accept => "accept-line" [b"\n", b"\r"],
    Back => "backward-char" [b"\x02", b"\x1bOD", b"\x1b[D"],
    Rubout => "backward-delete-char" [b"\x08", b"\x7f"],
    KillBack => "backward-kill-line" [b"\x18\x7f"],
    KillBackWord => "backward-kill-word" [b"\x1b\x08", b"\x1b\x7f"],
    BackWord => "backward-word" [b"\x1b[1;3D", b"\x1b[1;5D", b"\x1bb"],
    FirstHistory => "beginning-of-history" [b"\x1b<"],
    Home => "beginning-of-line" [b"\x01", b"\x1bOH", b"\x1b[H", b"\x1b[1~", b"\x1b[7~"],
    Paste => "bracketed-paste-begin" [b"\x1b[200~"],
    Clear => "clear-screen" [b"\x0c"],
    Complete => "complete" [b"\t", b"\x1b\x1b"],
    Copy => "copy-region-as-kill" [b"\x1bw"],
    Delete => "delete-char" [b"\x04", b"\x1b[3~"],
    LastHistory => "end-of-history" [b"\x1b>"],
    End => "end-of-line" [b"\x05", b"\x1bOF", b"\x1b[F", b"\x1b[4~", b"\x1b[8~"],
    SwapMark => "exchange-point-and-mark" [b"\x18\x18"],
    Forward => "forward-char" [b"\x06", b"\x1bOC", b"\x1b[C"],
    ForwardWord => "forward-word" [b"\x1b[1;3C", b"\x1b[1;5C", b"\x1bf"],
    Comment => "insert-comment" [b"\x1b#"],
    Kill => "kill-line" [b"\x0b"],
    Cut => "kill-region" [],
    KillWord => "kill-word" [b"\x1b[3;5~", b"\x1bd"],
    Redraw => "redraw-current-line" [],
    Emacs => "emacs-editing-mode" [],
    Vi => "vi-editing-mode" [],
    ViCommand => "vi-movement-mode" [],
    ViInsert => "vi-insertion-mode" [],
    ViAppend => "vi-append-mode" [],
    ViAppendEnd => "vi-append-eol" [],
    ViFirst => "vi-insert-beg" [],
    Down => "next-history" [b"\x0e", b"\x1bOB", b"\x1b[B"],
    List => "possible-completions" [b"\x1b=", b"\x1b?"],
    Up => "previous-history" [b"\x10", b"\x1bOA", b"\x1b[A"],
    Quote => "quoted-insert" [b"\x11", b"\x16"],
    Search => "reverse-search-history" [b"\x12"],
    Insert => "self-insert" [],
    Overwrite => "overwrite-mode" [b"\x1b[2~"],
    Mark => "set-mark" [b"\0", b"\x1b "],
    Transpose => "transpose-chars" [b"\x14"],
    Discard => "unix-line-discard" [b"\x15"],
    RubWord => "unix-word-rubout" [b"\x17"],
    Yank => "yank" [b"\x19"],
}

pub(crate) fn names() -> impl Iterator<Item = &'static str> {
    ACTIONS.iter().map(|(_, name, _)| *name)
}

#[derive(Clone)]
pub(super) enum Binding {
    Edit(Action),
    Macro(Vec<u8>),
    Shell(String),
    Removed,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Keymap {
    #[default]
    Emacs,
    Insert,
    Command,
}
impl Keymap {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "emacs" | "emacs-standard" => Ok(Self::Emacs),
            "vi" | "vi-move" | "vi-command" => Ok(Self::Command),
            "vi-insert" | "vi-insertion" => Ok(Self::Insert),
            _ => Err(format!("{name}: unsupported keymap")),
        }
    }
    fn defaults(self, action: Action, emacs: &'static [&'static [u8]]) -> &'static [&'static [u8]] {
        use Action::*;
        match self {
            Self::Emacs => emacs,
            Self::Insert => match action {
                ViCommand => &[b"\x1b"],
                Accept | Rubout | Delete | Complete | Clear | Paste | Quote | RubWord | Discard
                | Yank | Overwrite => emacs,
                Home => &[b"\x1bOH", b"\x1b[H"],
                End => &[b"\x1bOF", b"\x1b[F"],
                Back => &[b"\x1bOD", b"\x1b[D"],
                Forward => &[b"\x1bOC", b"\x1b[C"],
                Up | Down => emacs,
                _ => &[],
            },
            Self::Command => match action {
                ViInsert => &[b"i"],
                ViAppend => &[b"a"],
                ViAppendEnd => &[b"A"],
                ViFirst => &[b"I"],
                Home => &[b"0", b"^", b"\x1b[H"],
                End => &[b"$", b"\x1b[F"],
                Back => &[b"h", b"\x08", b"\x1b[D"],
                Forward => &[b"l", b" ", b"\x1b[C"],
                ForwardWord => &[b"w"],
                BackWord => &[b"b"],
                Delete => &[b"x", b"\x1b[3~"],
                Rubout => &[b"X"],
                Up => &[b"k", b"-", b"\x10", b"\x1b[A"],
                Down => &[b"j", b"+", b"\x0e", b"\x1b[B"],
                Kill => &[b"D", b"d$"],
                KillWord => &[b"dw"],
                KillBackWord => &[b"db"],
                Discard => &[b"d0"],
                Yank => &[b"P"],
                Search => &[b"/", b"\x12"],
                Accept | Clear | Paste => emacs,
                _ => &[],
            },
        }
    }
}
type KeyTable = BTreeMap<Vec<u8>, Binding>;
#[derive(Clone, Default)]
pub(crate) struct Bindings {
    keys: [Option<Rc<KeyTable>>; 3],
    pub(crate) map: Keymap,
    prefer_vi: Option<bool>,
    pub(super) disable_completion: bool,
    bracketed_paste: Option<bool>,
    timeout: Option<i32>,
    initialized: bool,
}
impl Bindings {
    pub(super) fn paste(&self) -> bool {
        self.bracketed_paste.unwrap_or(true)
    }
    pub(super) fn timeout(&self) -> i32 {
        self.timeout.unwrap_or(500)
    }
    fn set(&mut self, key: Vec<u8>, binding: Binding) {
        Rc::make_mut(self.keys[self.map as usize].get_or_insert_with(Default::default))
            .insert(key, binding);
    }
    pub(super) fn custom_prefix(&self, key: &[u8]) -> bool {
        self.keys[self.map as usize].as_deref().is_some_and(|keys| {
            keys.range::<[u8], _>((std::ops::Bound::Included(key), std::ops::Bound::Unbounded))
                .next()
                .is_some_and(|(k, _)| k.starts_with(key))
        })
    }
    pub(super) fn lookup(&self, seq: &[u8]) -> (Option<Binding>, bool) {
        let mut exact = self.keys[self.map as usize]
            .as_deref()
            .and_then(|k| k.get(seq))
            .cloned();
        let mut longer = self.keys[self.map as usize].as_deref().is_some_and(|keys| {
            keys.range::<[u8], _>((std::ops::Bound::Excluded(seq), std::ops::Bound::Unbounded))
                .next()
                .is_some_and(|(k, _)| k.starts_with(seq))
        });
        for &(action, _, keys) in ACTIONS {
            for &key in self.map.defaults(action, keys) {
                if key.starts_with(seq) {
                    if key.len() == seq.len() {
                        exact.get_or_insert(Binding::Edit(action));
                    } else {
                        longer = true;
                    }
                }
            }
        }
        (exact, longer)
    }
    fn keys_for(&self, action: Action, defaults: &'static [&'static [u8]]) -> Vec<Vec<u8>> {
        let mut result: Vec<Vec<u8>> = self
            .map
            .defaults(action, defaults)
            .iter()
            .map(|k| k.to_vec())
            .collect();
        if action == Action::Insert && self.map != Keymap::Command {
            result.extend((32..127).map(|c| vec![c]));
        }
        if let Some(map) = self.keys[self.map as usize].as_deref() {
            result.retain(|k| !map.contains_key(k));
            result.extend(map.iter().filter_map(|(k, v)| {
                matches!(v, Binding::Edit(a) if *a==action).then(|| k.clone())
            }));
            result.sort();
        }
        result
    }
}

fn quoted_end(s: &str) -> Option<usize> {
    let mut escaped = false;
    if !s.starts_with('"') {
        return None;
    }
    for (i, c) in s.char_indices().skip(1) {
        if !escaped && c == '"' {
            return Some(i);
        }
        escaped = !escaped && c == '\\';
    }
    None
}
fn decode(s: &str) -> Result<Vec<u8>, String> {
    let s = if let Some(end) = quoted_end(s) {
        &s[1..end]
    } else {
        s
    };
    let mut input = s.bytes().peekable();
    let mut out = Vec::new();
    while let Some(mut c) = input.next() {
        if c == b'\\' {
            c = input.next().ok_or("incomplete key escape")?;
            let mut control = false;
            while matches!(c, b'M' | b'm' | b'C' | b'c') && input.next_if_eq(&b'-').is_some() {
                if c.eq_ignore_ascii_case(&b'M') {
                    out.push(27);
                } else {
                    control = true;
                }
                c = input.next().ok_or("incomplete key escape")?;
                if c == b'\\' {
                    c = input.next().ok_or("incomplete key escape")?;
                } else {
                    break;
                }
            }
            c = if control {
                if c == b'?' { 127 } else { c & 31 }
            } else {
                match c {
                    b'e' | b'E' => 27,
                    b'n' => 10,
                    b'r' => 13,
                    b't' => 9,
                    b'a' => 7,
                    b'b' => 8,
                    b'f' => 12,
                    b'v' => 11,
                    b'x' | b'0'..=b'7' => {
                        let hex = c == b'x';
                        let radix = if hex { 16 } else { 8 };
                        let mut n = if hex { 0 } else { c - b'0' };
                        for _ in 0..2 {
                            let Some(d) = input.peek().and_then(|&b| (b as char).to_digit(radix))
                            else {
                                break;
                            };
                            input.next();
                            n = n.wrapping_mul(radix as u8).wrapping_add(d as u8);
                        }
                        n
                    }
                    _ => c,
                }
            }
        }
        out.push(c);
    }
    Ok(out)
}
fn encode(seq: &[u8], meta: bool) -> String {
    if meta
        && let Some(rest) = seq.strip_prefix(&[27])
        && !rest.is_empty()
    {
        let encoded = encode(rest, false);
        return format!("\"\\M-{}", &encoded[1..]);
    }
    let mut s = String::from("\"");
    for &c in seq {
        match c {
            27 => s.push_str("\\e"),
            0..=31 | 127 => {
                s.push_str("\\C-");
                s.push(if c == 127 {
                    '?'
                } else if c == 0 {
                    '@'
                } else if c >= 28 {
                    (c + 64) as char
                } else {
                    (c + 96) as char
                });
            }
            b'\\' | b'"' => {
                s.push('\\');
                s.push(c as char);
            }
            32..=126 => s.push(c as char),
            _ => s.push_str(&format!("\\{c:03o}")),
        }
    }
    s.push('"');
    s
}

fn configure(bindings: &mut Bindings, spec: &str, shell: bool) -> Result<(), String> {
    let spec = spec.trim();
    if !shell && let Some(set) = spec.strip_prefix("set ") {
        let (name, value) = set
            .trim()
            .split_once(char::is_whitespace)
            .ok_or("missing variable value")?;
        let value = value.trim();
        let on = || match value.to_ascii_lowercase().as_str() {
            "on" | "1" => Ok(true),
            "off" | "0" => Ok(false),
            _ => Err("expected on or off"),
        };
        match name {
            "disable-completion" => bindings.disable_completion = on()?,
            "enable-bracketed-paste" => bindings.bracketed_paste = Some(on()?),
            "keyseq-timeout" => {
                bindings.timeout = Some(
                    value
                        .parse::<i32>()
                        .ok()
                        .filter(|n| *n >= 0)
                        .ok_or("invalid key sequence timeout")?,
                )
            }
            "keymap" => bindings.map = Keymap::parse(value)?,
            "editing-mode" if matches!(value, "emacs" | "vi") => {
                bindings.prefer_vi = Some(value == "vi");
                bindings.map = if value == "vi" {
                    Keymap::Insert
                } else {
                    Keymap::Emacs
                };
            }
            _ => {
                return Err(format!(
                    "unsupported readline variable or value: {name} {value}"
                ));
            }
        }
        return Ok(());
    }
    let end = quoted_end(spec).ok_or("expected a quoted key sequence")?;
    let key = decode(&spec[..=end])?;
    if key.is_empty() {
        return Err("empty key sequence".into());
    }
    let value = spec[end + 1..]
        .trim_start()
        .strip_prefix(':')
        .ok_or("expected ':'")?
        .trim_start();
    let binding = if shell {
        Binding::Shell(if let Some(end) = quoted_end(value) {
            value[1..end].replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            value.to_owned()
        })
    } else if value.starts_with('"') {
        Binding::Macro(decode(value)?)
    } else {
        let name = value.split_whitespace().next().unwrap_or("");
        Binding::Edit(
            ACTIONS
                .iter()
                .find(|(_, n, _)| *n == name)
                .ok_or_else(|| format!("unknown readline function: {name}"))?
                .0,
        )
    };
    bindings.set(key, binding);
    Ok(())
}
fn load_file(session: &mut Session, path: &Path, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("inputrc include nesting limit".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut levels = Vec::new();
    let mut active = true;
    let mut error = None;
    for line in text.lines().map(str::trim) {
        if let Some(cond) = line.strip_prefix("$if ") {
            levels.push(active);
            let cond = cond.trim();
            active &= matches!(cond, "Bash" | "bash" | "Tonari" | "tonari" | "mode=emacs")
                || cond.strip_prefix("term=").is_some_and(|t| {
                    session
                        .get("TERM")
                        .is_some_and(|v| v == t || v.split('-').next() == Some(t))
                });
        } else if line == "$endif" {
            active = levels.pop().unwrap_or(true);
        } else if line == "$else" {
            active = levels.last().copied().unwrap_or(true) && !active;
        } else if active && !line.is_empty() && !line.starts_with('#') {
            let result = if let Some(name) = line.strip_prefix("$include ") {
                let name = name.trim();
                let file = if let Some(s) = name.strip_prefix("~/") {
                    PathBuf::from(session.get("HOME").unwrap_or_default()).join(s)
                } else {
                    path.parent().unwrap_or(Path::new(".")).join(name)
                };
                load_file(session, &file, depth + 1)
            } else {
                configure(&mut session.bindings, line, false)
            };
            if result.is_err() {
                error = result.err();
            }
        }
    }
    error.map_or(Ok(()), Err)
}
pub(crate) fn initialize(session: &mut Session) {
    if session.bindings.initialized {
        return;
    }
    session.bindings.initialized = true;
    let explicit = session.get("INPUTRC").filter(|v| !v.is_empty());
    let path = explicit
        .map(PathBuf::from)
        .or_else(|| {
            session
                .get("HOME")
                .map(|p| PathBuf::from(p).join(".inputrc"))
                .filter(|p| p.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("/etc/inputrc"));
    let _ = load_file(session, &path, 0);
    if let Some(vi) = session.bindings.prefer_vi.take() {
        session.vi = vi;
        session.emacs = !vi;
    }
}

pub(crate) fn builtin(session: &mut Session, args: &[String]) -> u8 {
    if !session.interactive {
        eprintln!(
            "{}: line {}: bind: warning: line editing not enabled",
            session.arg0,
            session.line()
        );
    }
    initialize(session);
    let old = session.bindings.map;
    let result = run(session, args);
    if args.iter().any(|s| s == "-m") {
        session.bindings.map = old;
    }
    if let Some(vi) = session.bindings.prefer_vi.take() {
        session.vi = vi;
        session.emacs = !vi;
    }
    match result {
        Ok(status) => status,
        Err(e) => {
            eprintln!("{}: line {}: bind: {e}", session.arg0, session.line());
            1
        }
    }
}
fn run(session: &mut Session, args: &[String]) -> Result<u8, String> {
    let meta = matches!(
        session.character_locale(),
        crate::session::CharacterLocale::Ascii
    );
    let dumb = session.get("TERM").as_deref() == Some("dumb");
    let encode = |s: &[u8]| encode(s, meta);
    let mut args = args.iter();
    let mut out = io::stdout().lock();
    let ioerr = |e: io::Error| e.to_string();
    while let Some(arg) = args.next() {
        let mut operand = || {
            args.next()
                .map(String::as_str)
                .ok_or_else(|| format!("{arg}: missing argument"))
        };
        match arg.as_str() {
            "--" => {
                for s in args.by_ref() {
                    configure(&mut session.bindings, s, false)?;
                }
            }
            "-m" => {
                let m = operand()?;
                session.bindings.map = Keymap::parse(m)?;
            }
            "-l" => {
                for name in names() {
                    writeln!(out, "{name}").map_err(ioerr)?;
                }
            }
            "-f" => load_file(session, Path::new(operand()?), 0)?,
            "-x" => configure(&mut session.bindings, operand()?, true)?,
            "-r" => session.bindings.set(decode(operand()?)?, Binding::Removed),
            "-q" | "-u" | "-p" | "-P" => {
                let target = matches!(arg.as_str(), "-q" | "-u")
                    .then(&mut operand)
                    .transpose()?;
                if let Some(name) = target
                    && !names().any(|n| n == name)
                {
                    return Err(format!("\u{60}{name}': unknown function name"));
                }
                for &(action, name, defaults) in ACTIONS {
                    if target.is_some_and(|s| s != name) {
                        continue;
                    }
                    let mut keys = session.bindings.keys_for(action, defaults);
                    if dumb && action == Action::Delete {
                        keys.retain(|k| k != b"\x1b[3~");
                    }
                    if arg == "-u" {
                        for key in keys {
                            session.bindings.set(key, Binding::Removed);
                        }
                        continue;
                    }
                    if arg == "-p" {
                        if keys.is_empty() {
                            writeln!(out, "# {name} (not bound)").map_err(ioerr)?;
                        }
                        for k in keys {
                            writeln!(out, "{}: {name}", encode(&k)).map_err(ioerr)?;
                        }
                    } else if keys.is_empty() {
                        writeln!(out, "{name} is not bound to any keys.").map_err(ioerr)?;
                        if arg == "-q" {
                            return Ok(1);
                        }
                    } else {
                        let names = keys
                            .iter()
                            .take(5)
                            .map(|k| encode(k))
                            .collect::<Vec<_>>()
                            .join(", ");
                        writeln!(
                            out,
                            "{name} can be invoked via {names}{}",
                            if keys.len() > 5 { ", ..." } else { "." }
                        )
                        .map_err(ioerr)?;
                    }
                }
            }
            "-s" | "-S" | "-X" => {
                for (key, value) in session.bindings.keys[session.bindings.map as usize]
                    .iter()
                    .flat_map(|m| m.iter())
                {
                    match value {
                        Binding::Shell(v) if arg == "-X" => {
                            writeln!(out, "{}: {}", encode(key), encode(v.as_bytes()))
                                .map_err(ioerr)?
                        }
                        Binding::Macro(v) if arg == "-s" => {
                            writeln!(out, "{}: {}", encode(key), encode(v)).map_err(ioerr)?
                        }
                        Binding::Macro(v) if arg == "-S" => {
                            writeln!(out, "{} outputs {}", encode(key), encode(v)).map_err(ioerr)?
                        }
                        _ => {}
                    }
                }
            }
            "-v" | "-V" => {
                let yes = |on| if on { "on" } else { "off" };
                let b = &session.bindings;
                for (name, value) in [
                    ("disable-completion", yes(b.disable_completion).to_owned()),
                    (
                        "editing-mode",
                        if session.vi { "vi" } else { "emacs" }.into(),
                    ),
                    (
                        "enable-bracketed-paste",
                        yes(b.bracketed_paste.unwrap_or(session.interactive)).into(),
                    ),
                    (
                        "keymap",
                        match b.map {
                            Keymap::Emacs => "emacs",
                            Keymap::Insert => "vi-insert",
                            Keymap::Command => "vi-command",
                        }
                        .into(),
                    ),
                    ("keyseq-timeout", b.timeout().to_string()),
                ] {
                    if arg == "-V" {
                        writeln!(out, "{name} is set to \u{60}{value}'")
                    } else {
                        writeln!(out, "set {name} {value}")
                    }
                    .map_err(ioerr)?;
                }
            }
            s if s.starts_with('-') => return Err(format!("{s}: unsupported option")),
            spec => configure(&mut session.bindings, spec, false)?,
        }
    }
    Ok(0)
}
