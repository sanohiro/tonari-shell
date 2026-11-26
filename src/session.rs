pub(crate) mod array;
mod assoc;
mod declare;
mod directories;
mod dynamic;
mod functions;
mod getopts;
pub(crate) mod mapfile;
mod options;
pub(crate) mod printf;
mod read;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use crate::parser::{FunctionDefinition, is_identifier};

#[derive(Debug, Clone, Copy)]
pub(crate) enum CharacterLocale {
    Ascii,
    Utf8,
    Turkic,
}

impl CharacterLocale {
    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "" | "C" | "POSIX" => Self::Ascii,
            _ if matches!(name.split(['_', '.', '@', '-']).next(), Some("tr" | "az")) => {
                Self::Turkic
            }
            _ => Self::Utf8,
        }
    }
}

#[derive(Clone, Default)]
struct Variable {
    // A binding keeps its name in saved scopes. The process environment is
    // immutable until exec or entry into a new shell session.
    inherited: std::cell::OnceCell<bool>,
    dynamic: bool,
    temporary: bool,
    letter_case: Option<bool>,
    nameref: bool,
    readonly: bool,
    integer: bool,
    array_assigned: bool,
    elements: Option<Rc<array::Elements>>,
    declared: bool,
    value: Option<String>,
    exported: bool,
}

#[derive(Clone)]
struct ShellPid(u32, libc::pid_t);

impl Default for ShellPid {
    fn default() -> Self {
        Self(std::process::id(), unsafe { libc::getppid() })
    }
}

// Single commands update PIPESTATUS without allocating. Pipelines reuse the
// storage for stages beyond the first.
#[derive(Clone, Default)]
pub struct PipelineStatus {
    first: Option<u8>,
    rest: Vec<u8>,
}

impl PipelineStatus {
    pub fn single(&mut self, code: u8) {
        self.first = Some(code);
        self.rest.clear();
    }
    pub fn push(&mut self, code: u8) {
        if self.first.is_none() {
            self.first = Some(code);
        } else {
            self.rest.push(code);
        }
    }
    pub fn clear(&mut self) {
        self.first = None;
        self.rest.clear();
    }
}

#[derive(Clone)]
pub(crate) struct Function {
    pub definition: Rc<FunctionDefinition>,
    pub source: Rc<str>,
}

#[derive(Clone, Default)]
struct DirectoryVisits {
    paths: Vec<PathBuf>,
    current: usize,
}

impl DirectoryVisits {
    fn record(&mut self, from: Option<PathBuf>, to: &Path) {
        self.paths.truncate(self.current + 1);
        if let Some(from) = from
            && self.paths.last() != Some(&from)
        {
            self.paths.push(from);
        }
        self.paths.push(to.to_owned());
        if self.paths.len() > 128 {
            self.paths.drain(..self.paths.len() - 128);
        }
        self.current = self.paths.len() - 1;
    }
}

#[derive(Clone, Default)]
pub struct Session {
    // Only changed variables are stored here. Simple commands inherit the
    // original process environment without building a copy on startup.
    variables: BTreeMap<String, Variable>,
    dynamic: dynamic::Dynamic,
    has_namerefs: bool,
    reference_depth: std::cell::Cell<u8>,
    opt_offset: usize,
    logical_cwd: std::cell::OnceCell<PathBuf>,
    dir_stack: Vec<String>,
    visits: Option<Rc<DirectoryVisits>>,
    shell_pid: ShellPid,
    pub(crate) started_at: libc::time_t,
    pub interactive: bool,
    pub login: bool,
    pub posix: bool,
    pub logout_started: bool,
    pub allexport: bool,
    pub notify: bool,
    pub noexec: bool,
    pub ignoreeof: bool,
    pub xtrace: bool,
    pub onecmd: bool,
    pub subshell_depth: u32,
    pub histexpand: bool,
    pub emacs: bool,
    pub vi: bool,
    pub history_enabled: bool,
    pub verbose: bool,
    pub(crate) history: Option<Rc<crate::history::Entries>>,
    pub(crate) initial_history: Option<Rc<crate::history::InitialJournal>>,
    pub command_number: usize,
    pub(crate) evaluating: bool,
    pub(crate) trace_depth: u32,
    pub(crate) signal_line: Option<usize>,
    pub command_string: bool,
    pub command_history_expansion: bool,
    pub stdin_script: bool,
    pub tested_depth: u32,
    pub errexit: bool,
    pub errtrace: bool,
    pub functrace: bool,
    pub nounset: bool,
    pub pipefail: bool,
    pub pipe_status: PipelineStatus,
    pub stdin_redirected: bool,
    pub noglob: bool,
    pub noclobber: bool,
    pub shopt: options::ShellOptions,
    pub(crate) external_env: std::cell::RefCell<Option<Rc<crate::execute::Environment>>>,
    pub(crate) eval_cache: crate::input::EvalCache,
    pub regex: [Option<Rc<crate::test_expression::Compiled>>; 2],
    pub(crate) coprocesses: Vec<Rc<crate::execute::coprocess::Coprocess>>,
    pub substitutions: crate::execute::process_substitution::Substitutions,
    pub jobs: crate::jobs::Jobs,
    pub traps: crate::traps::Traps,
    pub hashes: crate::execute::hash::Table,
    pub last_background: Option<libc::pid_t>,
    pub arg0: String,
    pub(crate) last_argument: Option<String>,
    pub positional: std::vec::IntoIter<String>,
    pub source_depth: u32,
    pub function_depth: u32,
    pub function_names: Vec<String>,
    pub source_names: Vec<Rc<str>>,
    pub current_command: Option<crate::parser::Location>,
    pub debug_args: Vec<Vec<String>>,
    pub traced_functions: std::collections::BTreeSet<String>,
    pub call_lines: Vec<usize>,
    pub function_environment: BTreeMap<String, Option<String>>,
    pub loop_depth: u32,
    pub editor_kill: String,
    pub editor_queue: std::collections::VecDeque<u8>,
    pub bindings: crate::editor::bindings::Bindings,
    pub disabled_builtins: std::collections::BTreeSet<String>,
    pub aliases: crate::alias::Aliases,
    pub completions: crate::completion_spec::Registry,
    pub readonly_functions: std::collections::BTreeSet<String>,
    pub(crate) functions: BTreeMap<String, Function>,
    locals: Vec<Bindings>,
}

#[derive(Clone, Default)]
pub struct Bindings(BTreeMap<String, Option<Variable>>, Option<usize>, bool);

enum DirectoryError {
    NoPath(io::Error),
    Other(io::Error),
}

impl From<io::Error> for DirectoryError {
    fn from(e: io::Error) -> Self {
        Self::Other(e)
    }
}

impl Session {
    fn convert_case(&self, mut text: String, letter_case: Option<bool>) -> String {
        match letter_case {
            Some(upper)
                if text.is_ascii()
                    && !text.as_bytes().contains(&if upper { b'i' } else { b'I' }) =>
            {
                if upper {
                    text.make_ascii_uppercase();
                } else {
                    text.make_ascii_lowercase();
                }
                text
            }
            Some(upper) => {
                crate::pattern::case_chars(text, upper, true, None, self.character_locale())
            }
            None => text,
        }
    }

    pub(crate) fn character_locale(&self) -> CharacterLocale {
        const NAMES: [&str; 3] = ["LC_ALL", "LC_CTYPE", "LANG"];
        static INHERITED: std::sync::OnceLock<CharacterLocale> = std::sync::OnceLock::new();
        if !NAMES.iter().any(|name| self.variables.contains_key(*name)) {
            return *INHERITED.get_or_init(|| {
                let name = NAMES
                    .iter()
                    .find_map(|k| std::env::var(k).ok().filter(|s| !s.is_empty()));
                CharacterLocale::from_name(name.as_deref().unwrap_or(""))
            });
        }
        let name = NAMES
            .iter()
            .find_map(|name| self.get(name).filter(|s| !s.is_empty()));
        CharacterLocale::from_name(name.as_deref().unwrap_or(""))
    }

    pub fn variable_names(&self, prefix: &str) -> Vec<String> {
        let mut names: std::collections::BTreeSet<_> = std::env::vars_os()
            .filter_map(|(k, _)| k.into_string().ok())
            .filter(|k| k.starts_with(prefix) && is_identifier(k))
            .collect();
        names.extend(
            [
                "PWD",
                "IFS",
                "PATH",
                "OPTIND",
                "OPTERR",
                "UID",
                "EUID",
                "PPID",
                "BASHOPTS",
                "SHELLOPTS",
                "BASH",
                "BASH_VERSION",
                "BASH_VERSINFO",
                "SECONDS",
                "RANDOM",
                "SRANDOM",
                "HOSTTYPE",
                "HOSTNAME",
                "PS4",
                "OSTYPE",
                "MACHTYPE",
            ]
            .into_iter()
            .filter(|k| k.starts_with(prefix))
            .map(str::to_owned),
        );
        use std::ops::Bound::{Included, Unbounded};
        for (key, var) in self
            .variables
            .range::<str, _>((Included(prefix), Unbounded))
        {
            if !key.starts_with(prefix) {
                break;
            }
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
        if self.allexport {
            flags.push('a');
        }
        if self.notify {
            flags.push('b');
        }
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
        if self.jobs.monitor {
            flags.push('m');
        }
        if self.noexec {
            flags.push('n');
        }
        if self.onecmd {
            flags.push('t');
        }
        if self.nounset {
            flags.push('u');
        }
        if self.verbose {
            flags.push('v');
        }
        if self.xtrace {
            flags.push('x');
        }
        if self.shopt.braceexpand {
            flags.push('B');
        }
        if self.noclobber {
            flags.push('C');
        }
        if self.errtrace {
            flags.push('E');
        }
        if self.histexpand {
            flags.push('H');
        }
        if self.functrace {
            flags.push('T');
        }
        if self.command_string {
            flags.push('c');
        }
        if self.stdin_script {
            flags.push('s');
        }
        flags
    }

    pub fn printf(&mut self, args: &[String]) -> u8 {
        printf::run(self, args, None)
    }

    pub fn read(&mut self, args: &[String], here: Option<(i32, &[u8])>) -> u8 {
        read::run(self, args, here)
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
            if self.posix {
                eprintln!("tonari: shift: shift count out of range");
            }
            return 1;
        }
        if count > 0 {
            self.positional.nth(count as usize - 1);
        }
        0
    }

    pub fn set(&mut self, words: &mut Vec<String>) -> u8 {
        let args = words.get(1..).unwrap_or_default();
        if args.is_empty() {
            let mut out = io::BufWriter::new(io::stdout().lock());
            let locale = self.character_locale();
            for name in self.variable_names("") {
                let result = if let Some(var) = self.variables.get(&name)
                    && var.elements.is_some()
                {
                    if !var.array_assigned {
                        continue;
                    }
                    write!(out, "{name}").and_then(|_| print_variable_value(&mut out, var))
                } else if let Some(value) = self
                    .variables
                    .get(&name)
                    .filter(|v| !v.dynamic)
                    .and_then(|v| v.value.as_deref())
                    .map(Cow::Borrowed)
                    .or_else(|| self.get(&name).map(Cow::Owned))
                {
                    writeln!(out, "{name}={}", set_value(&value, self.posix, locale))
                } else {
                    continue;
                };
                if result.is_err() {
                    return 1;
                }
            }
            if !self.posix {
                for name in self.functions.keys() {
                    if self.print_function(&mut out, name).is_err() {
                        return 1;
                    }
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
            if opt == "+" {
                i += 1;
                continue;
            }
            if opt == "-" {
                self.xtrace = false;
                self.verbose = false;
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
                    'E' => self.errtrace = enable,
                    'a' => self.allexport = enable,
                    'b' => self.notify = enable,
                    'n' => self.noexec = enable && !self.interactive,
                    't' => self.onecmd = enable,
                    'x' => self.xtrace = enable,
                    'H' => {
                        self.histexpand = enable;
                        self.command_history_expansion = enable;
                    }
                    'T' => self.functrace = enable,
                    'C' => self.noclobber = enable,
                    'B' => self.shopt.braceexpand = enable,
                    'f' => self.noglob = enable,
                    'h' => self.hashes.enabled = enable,
                    'm' => self.jobs.monitor = enable,
                    'u' => self.nounset = enable,
                    'v' => self.verbose = enable,
                    'o' => {
                        let Some(name) = args.get(i) else {
                            let mut out = io::BufWriter::with_capacity(512, io::stdout().lock());
                            for (name, on) in self.options(true) {
                                let result = if enable {
                                    writeln!(out, "{name:15}\t{}", if on { "on" } else { "off" })
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
                            self.option_changed(name, enable);
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
            words.drain(..i + 1);
            self.positional = std::mem::take(words).into_iter();
        }
        0
    }

    pub fn is_nameref(&self, name: &str) -> bool {
        self.has_namerefs && self.variables.get(name).is_some_and(|v| v.nameref)
    }

    pub fn reference_name<'a>(&self, name: &'a str) -> Result<Cow<'a, str>, &'static str> {
        if !self.has_namerefs {
            return Ok(Cow::Borrowed(name));
        }
        let mut result = Cow::Borrowed(name);
        for _ in 0..32 {
            let (base, suffix) = array::subscript(&result)
                .map_or((result.as_ref(), ""), |(base, _)| {
                    (base, &result[base.len()..])
                });
            let Some(v) = self.variables.get(base).filter(|v| v.nameref) else {
                return Ok(result);
            };
            let Some(target) = v.value.as_deref().filter(|t| !t.is_empty()) else {
                return Ok(result);
            };
            if !suffix.is_empty() && array::subscript(target).is_some() {
                return Err("invalid indexed name reference");
            }
            result = Cow::Owned(format!("{target}{suffix}"));
        }
        Err("circular name reference")
    }

    pub fn get(&self, name: &str) -> Option<String> {
        if name == "_" {
            return self
                .last_argument
                .clone()
                .or_else(|| std::env::var("_").ok())
                .or_else(|| std::env::args_os().next()?.into_string().ok());
        }
        if let Some(base) = name.strip_prefix('\u{1}') {
            let resolved = self.reference_name(base).ok()?;
            let base = resolved.as_ref();
            if let Some(map) = self.variables.get(base).and_then(Variable::associative) {
                return Some(
                    map.ordered()
                        .into_iter()
                        .map(|(key, _)| key)
                        .collect::<Vec<_>>()
                        .join("\0"),
                );
            }
            return Some(
                self.indices(base)
                    .into_iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join("\0"),
            );
        }
        if let Cow::Owned(target) = self.reference_name(name).ok()? {
            let depth = self.reference_depth.get();
            if depth >= 32 {
                return None;
            }
            self.reference_depth.set(depth + 1);
            let value = self.get(&target);
            self.reference_depth.set(depth);
            return value;
        }
        if let Some((base, index)) = array::subscript(name) {
            if self.is_associative(base) {
                return self.assoc_value(base, index).map(str::to_owned);
            }
            let i = self
                .resolve_index(base, index)
                .map_err(|error| {
                    if error == "array subscript out of range" {
                        eprintln!("tonari: {base}: bad array subscript");
                    }
                })
                .ok()?;
            return self.element(base, i);
        }
        if matches!(name, "BASHOPTS" | "SHELLOPTS") {
            return Some(self.option_list(name == "SHELLOPTS"));
        }
        if name == "PIPESTATUS" {
            return self.element(name, 0);
        }
        if let Some(var) = self.variables.get(name) {
            if var.dynamic {
                return self.dynamic_number(name).map(|v| v.to_string());
            }
            return var
                .elements
                .as_ref()
                .map_or_else(|| var.value.clone(), |a| a.get(&0).map(str::to_owned));
        }
        if matches!(
            name,
            "BASH_SOURCE" | "BASH_LINENO" | "BASH_ARGC" | "BASH_ARGV" | "FUNCNAME" | "DIRSTACK"
        ) {
            return self.element(name, 0);
        }
        self.inherited_value(name)
    }

    fn inherited_value(&self, name: &str) -> Option<String> {
        match name {
            "RANDOM" | "SECONDS" | "SRANDOM" => self.dynamic_number(name).map(|v| v.to_string()),
            "BASH_SUBSHELL" => Some(self.subshell_depth.to_string()),
            "BASH_ARGV0" => Some(self.arg0.clone()),
            "EPOCHSECONDS" | "EPOCHREALTIME" => {
                let dt = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?;
                Some(if name == "EPOCHSECONDS" {
                    dt.as_secs().to_string()
                } else {
                    format!("{}.{:06}", dt.as_secs(), dt.subsec_micros())
                })
            }
            "LINENO" => Some(self.line().to_string()),
            "BASH_COMMAND" => self.current_command.as_ref().map(|c| c.text().to_owned()),
            "EUID" => Some(unsafe { libc::geteuid() }.to_string()),
            "UID" => Some(unsafe { libc::getuid() }.to_string()),
            "PPID" => Some(self.shell_pid.1.to_string()),
            "BASHOPTS" | "SHELLOPTS" => Some(self.option_list(name == "SHELLOPTS")),
            "PWD" => self
                .logical_directory()
                .ok()?
                .into_os_string()
                .into_string()
                .ok(),
            "BASH_VERSION" => Some("5.2.0(1)-tonari".into()),
            "BASH_VERSINFO" => self.element(name, 0),
            "SHELL" => std::env::var(name).ok().or_else(crate::word::account_shell),
            "HOSTNAME" => std::env::var(name).ok().or_else(|| {
                static HOST: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
                HOST.get_or_init(|| {
                    let mut host = [0u8; 256];
                    (unsafe { libc::gethostname(host.as_mut_ptr().cast(), host.len() - 1) } == 0)
                        .then(|| {
                            String::from_utf8_lossy(
                                &host[..host.iter().position(|&c| c == 0).unwrap()],
                            )
                            .into_owned()
                        })
                })
                .clone()
            }),
            "PS4" => Some(std::env::var(name).unwrap_or_else(|_| "+ ".into())),
            "HOSTTYPE" => {
                Some(std::env::var(name).unwrap_or_else(|_| std::env::consts::ARCH.into()))
            }
            "OSTYPE" => Some(std::env::var(name).unwrap_or_else(|_| Self::os_type().into())),
            "MACHTYPE" => Some(Self::machine_type()),
            "BASH" => std::env::current_exe()
                .ok()?
                .into_os_string()
                .into_string()
                .ok(),
            "OPTIND" | "OPTERR" => Some("1".into()),
            "IFS" => Some(" \t\n".into()),
            "PATH" => Some(std::env::var(name).unwrap_or_else(|_| {
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into()
            })),
            _ => std::env::var(name).ok(),
        }
    }

    fn os_type() -> &'static str {
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux-gnu"
        }
    }

    fn machine_type() -> String {
        let system = if cfg!(target_os = "macos") {
            "apple-darwin"
        } else if cfg!(target_env = "musl") {
            "unknown-linux-musl"
        } else {
            "unknown-linux-gnu"
        };
        format!("{}-{system}", std::env::consts::ARCH)
    }

    pub fn readonly(&self, name: &str) -> bool {
        let Ok(target) = self.reference_name(name) else {
            return true;
        };
        let base = array::subscript(&target).map_or(target.as_ref(), |(base, _)| base);
        Self::builtin_readonly(base) || self.variables.get(base).is_some_and(|var| var.readonly)
    }

    fn builtin_readonly(name: &str) -> bool {
        matches!(
            name,
            "UID" | "EUID" | "PPID" | "BASHOPTS" | "SHELLOPTS" | "BASH_VERSINFO"
        )
    }

    pub fn initialize_shell_level(&mut self) {
        if unsafe { libc::getenv(c"PWD".as_ptr()) }.is_null()
            && let Some(directory) = self.get("PWD")
        {
            self.assign("PWD", &directory);
        }
        let inherited = std::env::var("SHLVL")
            .ok()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let level = inherited.saturating_add(1);
        let level = if level >= 1000 {
            eprintln!("tonari: warning: shell level ({level}) too high, resetting to 1");
            1
        } else {
            level.max(0)
        };
        self.variables.insert(
            "SHLVL".into(),
            Variable {
                value: Some(level.to_string()),
                declared: true,
                exported: true,
                ..Variable::default()
            },
        );
    }

    pub fn assign(&mut self, name: &str, value: &str) {
        self.assign_value(name, Cow::Borrowed(value));
    }

    fn ordinary_assignment(name: &str) -> bool {
        !matches!(
            name,
            "BASH_ARGV0"
                | "GLOBIGNORE"
                | "HISTSIZE"
                | "HISTFILESIZE"
                | "OPTIND"
                | "PATH"
                | "PIPESTATUS"
        )
    }

    pub(crate) fn can_append_scalar(&self, name: &str) -> bool {
        Self::ordinary_assignment(name)
            && !Self::builtin_readonly(name)
            && self.variables.get(name).is_some_and(|v| {
                !v.dynamic
                    && v.letter_case.is_none()
                    && !v.readonly
                    && !v.integer
                    && !v.nameref
                    && v.elements.is_none()
                    && v.value.is_some()
            })
    }

    // The caller checked integer and readonly attributes and resolved references.
    pub(crate) fn append_plain_scalar(&mut self, name: &str, suffix: &str) -> bool {
        if !Self::ordinary_assignment(name) {
            return false;
        }
        let Some(var) = self.variables.get_mut(name) else {
            return false;
        };
        if var.elements.is_some() || var.nameref || var.letter_case.is_some() || var.dynamic {
            return false;
        }
        let Some(value) = &mut var.value else {
            return false;
        };
        value.push_str(suffix);
        var.declared = true;
        var.exported |= self.allexport;
        true
    }

    pub(crate) fn assign_value(&mut self, name: &str, mut value: Cow<'_, str>) {
        match self.reference_name(name) {
            Ok(Cow::Owned(target)) => {
                if let Err(e) = self.assign_target(&target, &value) {
                    eprintln!("tonari: {name}: {e}");
                }
                return;
            }
            Err(e) => {
                eprintln!("tonari: {name}: {e}");
                return;
            }
            _ => {}
        }
        if self.readonly(name) {
            return;
        }
        if matches!(name, "HISTSIZE" | "HISTFILESIZE") {
            self.apply_history_limit(name, &value);
        }
        if name == "BASH_ARGV0" {
            self.arg0 = value.to_string();
        }
        if name == "GLOBIGNORE" {
            self.shopt.glob.set_ignore(Some(&value));
        }
        if name == "OPTIND" {
            self.opt_offset = 0;
            value = Cow::Owned(value.trim().parse::<i64>().unwrap_or(0).to_string());
        }
        if name == "PATH" {
            self.hashes.clear();
        }
        let live = dynamic::is_dynamic(name) && self.variables.get(name).is_none_or(|v| v.dynamic);
        if let Some(upper) = self.variables.get(name).and_then(|v| v.letter_case) {
            value = Cow::Owned(self.convert_case(value.into_owned(), Some(upper)));
        }
        if live {
            self.set_dynamic(name, &value);
        }
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(elements) = var.elements.as_mut().map(Rc::make_mut) {
                elements.insert(0, value.into_owned());
                var.array_assigned = true;
            } else if let (Some(current), Cow::Borrowed(text)) = (&mut var.value, &value) {
                (*text).clone_into(current);
            } else {
                var.value = Some(value.into_owned());
            }
            var.declared = true;
            var.exported |= self.allexport;
            return;
        }
        let exported = self.allexport || self.is_exported(name);
        self.variables.insert(
            name.into(),
            Variable {
                letter_case: None,
                temporary: false,
                dynamic: live,
                inherited: Default::default(),
                readonly: false,
                nameref: false,
                integer: live,
                array_assigned: false,
                elements: None,
                declared: true,
                value: Some(value.into_owned()),
                exported,
            },
        );
    }

    // An existing plain arithmetic variable can keep the decimal string's
    // capacity. Special variables retain their ordinary assignment hooks.
    pub(crate) fn write_scalar_decimal(
        &mut self,
        name: &str,
        value: i64,
    ) -> Option<Result<(), &'static str>> {
        if !Self::ordinary_assignment(name) || Self::builtin_readonly(name) {
            return None;
        }
        let var = self.variables.get_mut(name)?;
        if var.dynamic || var.nameref || var.elements.is_some() {
            return None;
        }
        if var.readonly {
            return Some(Err("readonly variable"));
        }
        let text = var.value.as_mut()?;
        text.clear();
        use std::fmt::Write;
        write!(text, "{value}").unwrap();
        var.declared = true;
        var.exported |= self.allexport;
        Some(Ok(()))
    }

    // Only an existing scalar with ordinary assignment semantics can be
    // updated in place. A changed attribute takes effect on the very next write.
    pub(crate) fn assign_existing_literal(&mut self, key: &str, value: &str) -> bool {
        let (name, append) = key
            .strip_suffix('+')
            .map_or((key, false), |name| (name, true));
        if matches!(
            name,
            "OPTIND" | "GLOBIGNORE" | "PATH" | "BASH_ARGV0" | "PIPESTATUS"
        ) || Self::builtin_readonly(name)
        {
            return false;
        }
        let Some(var) = self.variables.get_mut(name) else {
            return false;
        };
        if var.dynamic
            || var.letter_case.is_some()
            || var.readonly
            || var.nameref
            || var.elements.is_some()
        {
            return false;
        }
        let Some(current) = var.value.as_mut() else {
            return false;
        };
        if var.integer {
            let Some(rhs) = Self::plain_decimal(value) else {
                return false;
            };
            let result = if append {
                let Some(lhs) = Self::plain_decimal(current) else {
                    return false;
                };
                lhs.wrapping_add(rhs)
            } else {
                rhs
            };
            current.clear();
            use std::fmt::Write;
            write!(current, "{result}").unwrap();
        } else if append {
            current.push_str(value);
        } else {
            value.clone_into(current);
        }
        var.exported |= self.allexport;
        var.declared = true;
        true
    }

    pub(crate) fn remember_literal_argument(&mut self, words: &[String]) {
        let arg = words.last().map_or("", String::as_str);
        if self.last_argument.as_deref() != Some(arg) {
            let slot = self.last_argument.get_or_insert_with(String::new);
            slot.clear();
            slot.push_str(arg);
        }
    }

    pub(crate) fn stored_scalar(&self, name: &str) -> Option<&str> {
        if name == "_" {
            return self.last_argument.as_deref();
        }
        if matches!(name, "BASHPID" | "BASHOPTS" | "SHELLOPTS" | "PIPESTATUS") {
            return None;
        }
        let var = self.variables.get(name)?;
        if var.dynamic || var.nameref || var.elements.is_some() {
            return None;
        }
        var.value.as_deref()
    }

    pub(crate) fn scalar_decimal(&self, name: &str) -> Option<i64> {
        let var = self.variables.get(name)?;
        if var.dynamic || var.nameref || var.elements.is_some() {
            return None;
        }
        Self::plain_decimal(var.value.as_deref()?)
    }

    pub(crate) fn plain_decimal(value: &str) -> Option<i64> {
        if value == "0" {
            return Some(0);
        }
        if !matches!(
            value.trim_start_matches('-').as_bytes().first(),
            Some(b'1'..=b'9')
        ) {
            return None;
        }
        value.parse().ok()
    }

    pub fn integer(&self, name: &str) -> bool {
        self.variables
            .get(name)
            .map_or_else(|| dynamic::is_dynamic(name), |v| v.integer)
    }

    pub(crate) fn integer_value(
        &mut self,
        old: Option<&str>,
        expression: &str,
        append: bool,
    ) -> Result<String, &'static str> {
        let old = if append {
            crate::arithmetic::run(old.unwrap_or("0"), self)?
        } else {
            0
        };
        Ok(old
            .wrapping_add(crate::arithmetic::run(expression, self)?)
            .to_string())
    }

    pub(crate) fn is_exported(&self, name: &str) -> bool {
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
            names.contains(&"OPTIND").then_some(self.opt_offset),
            false,
        )
    }

    // Special builtins retain their exported prefix bindings in POSIX mode.
    // Move the prepared strings and avoid snapshotting variables for restoration.
    pub fn persist_prefix(&mut self, assignments: &mut Vec<(String, String)>) {
        for (name, value) in assignments.drain(..) {
            if name == "OPTIND" {
                self.opt_offset = 0;
            }
            if name == "GLOBIGNORE" {
                self.shopt.glob.set_ignore(Some(&value));
            }
            if name == "PATH" {
                self.hashes.clear();
            }
            self.variables.insert(
                name,
                Variable {
                    declared: true,
                    exported: true,
                    value: Some(value),
                    ..Variable::default()
                },
            );
        }
    }

    pub fn bind(&mut self, assignments: &[(String, String)]) -> Bindings {
        let mut saved = Bindings::default();
        for (name, value) in assignments {
            if self.readonly(name) {
                continue;
            }
            if name == "GLOBIGNORE" {
                self.shopt.glob.set_ignore(Some(value));
            }
            if name == "OPTIND" {
                saved.1.get_or_insert(self.opt_offset);
                self.opt_offset = 0;
            }
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
                    letter_case: None,
                    temporary: true,
                    dynamic: false,
                    inherited: Default::default(),
                    readonly: false,
                    nameref: false,
                    integer: false,
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
        if let Some(offset) = bindings.1 {
            self.opt_offset = offset;
        }
        for (name, original) in bindings.0 {
            if name == "GLOBIGNORE" {
                self.shopt
                    .glob
                    .set_ignore(original.as_ref().and_then(|v| v.value.as_deref()));
            }
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
    ) -> impl Iterator<Item = (&'a str, Option<Cow<'a, str>>)> {
        self.variables
            .iter()
            .filter_map(|(name, var)| {
                if var.exported && var.elements.is_none() && var.value.is_some() {
                    let value = match name.as_str() {
                        "BASHOPTS" | "SHELLOPTS" => {
                            Some(Cow::Owned(self.option_list(name == "SHELLOPTS")))
                        }
                        _ => var.value.as_deref().map(Cow::Borrowed),
                    };
                    Some((name.as_str(), value))
                } else if *var
                    .inherited
                    .get_or_init(|| std::env::var_os(name).is_some())
                {
                    Some((name.as_str(), None))
                } else {
                    None
                }
            })
            .chain(
                self.function_environment
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_deref().map(Cow::Borrowed))),
            )
            .chain(
                assignments
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(Cow::Borrowed(v.as_str())))),
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

    pub fn line(&self) -> usize {
        self.signal_line
            .unwrap_or_else(|| self.current_command.as_ref().map_or(1, |c| c.line))
    }

    fn declaration_scope(&self, name: &str) -> Option<usize> {
        if self.function_depth == 0 {
            return None;
        }
        self.locals
            .iter()
            .rposition(|scope| !scope.2 || scope.0.contains_key(name))
    }

    pub fn enter_scope(&mut self, bindings: &mut Bindings, temporary: bool) {
        let mut scope = std::mem::take(bindings);
        scope.2 = temporary;
        self.locals.push(scope);
    }

    pub fn leave_scope(&mut self) {
        let scope = self.locals.pop().expect("variable scope");
        self.restore(scope);
    }

    pub fn enter_function(&mut self, bindings: &mut Bindings) {
        self.enter_scope(bindings, false);
        self.function_depth += 1;
    }

    pub fn leave_function(&mut self) {
        self.leave_scope();
        self.function_depth -= 1;
    }

    pub fn export(&mut self, args: &[String], bindings: &mut Bindings) -> u8 {
        let mut functions = false;
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
                    'f' => functions = true,
                    'p' => {}
                    _ => {
                        eprintln!("tonari: export: unsupported option {opt}");
                        return 2;
                    }
                }
            }
            i += 1;
        }
        if functions {
            let mut status = 0;
            if i == args.len() {
                let mut out = io::stdout().lock();
                for name in self.functions.keys().filter(|n| self.function_exported(n)) {
                    if self.print_function(&mut out, name).is_err() {
                        return 1;
                    }
                }
            } else {
                for name in &args[i..] {
                    if !remove && name.contains('/') && self.functions.contains_key(name) {
                        eprintln!("tonari: export: {name}: cannot export");
                        status = 1;
                        continue;
                    }
                    if let Err(e) = self.export_function(name, !remove) {
                        eprintln!("tonari: export: {name}: {e}");
                        status = 1;
                    }
                }
            }
            return status;
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
            let readonly = self.readonly(name);
            if readonly && value.is_some() {
                eprintln!("tonari: export: {name}: readonly variable");
                status = 1;
                continue;
            }
            if name == "PATH" && value.is_some() {
                self.hashes.clear();
            }
            let integer = self.integer(name);
            let normalized;
            let (value, append) = if integer && let Some(value) = value {
                let original = append.then(|| self.get(name)).flatten();
                match self.integer_value(original.as_deref(), value, append) {
                    Ok(v) => {
                        normalized = v;
                        (Some(normalized.as_str()), false)
                    }
                    Err(e) => {
                        eprintln!("tonari: export: {e}");
                        status = 1;
                        continue;
                    }
                }
            } else {
                (value, append)
            };
            if let Some(text) = value {
                let text = if append {
                    Cow::Owned(self.get(name).unwrap_or_default() + text)
                } else {
                    Cow::Borrowed(text)
                };
                self.assign_value(name, text);
            }
            // Export takes a snapshot. Launching successive child processes
            // must not advance the random sequence on behalf of the parent.
            let snapshot = self
                .variables
                .get(name)
                .filter(|var| var.dynamic)
                .and_then(|_| self.dynamic_number(name));
            if let Some(var) = self.variables.get_mut(name) {
                if let Some(number) = snapshot {
                    var.value = Some(number.to_string());
                }
                var.exported = !remove;
                var.declared = true;
            } else {
                let mut var = self.declared_variable(name, false).unwrap_or_default();
                var.exported = !remove;
                var.declared = true;
                self.variables.insert(name.into(), var);
            }
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
                if matches!(name.as_str(), "BASHOPTS" | "SHELLOPTS") {
                    let current = self.declared_variable(&name, false).unwrap();
                    print_declaration(&mut out, &name, &current)?;
                } else {
                    print_declaration(&mut out, &name, var)?;
                }
                continue;
            }
            print_declaration(
                &mut out,
                &name,
                &Variable {
                    letter_case: None,
                    temporary: false,
                    dynamic: false,
                    inherited: Default::default(),
                    readonly: false,
                    nameref: false,
                    integer: false,
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

    pub(crate) fn unset_variable(&mut self, name: &str) {
        if self.readonly(name) {
            return;
        }
        if name == "GLOBIGNORE" {
            self.shopt.glob.set_ignore(None);
        }
        if name == "OPTIND" {
            self.opt_offset = 0;
        }
        if name == "PATH" {
            self.hashes.clear();
        }
        if let Some(index) = self
            .locals
            .iter()
            .rposition(|scope| scope.0.contains_key(name))
            && (self.variables.get(name).is_some_and(|v| v.temporary)
                || !self.shopt.localvar_unset && index + 1 < self.locals.len())
        {
            // Unsetting a caller's local exposes the binding it was shadowing.
            let scope = &mut self.locals[index];
            if let Some(original) = scope.0.remove(name).flatten() {
                self.variables.insert(name.into(), original);
            } else {
                self.variables.remove(name);
            }
            if name == "OPTIND"
                && let Some(offset) = scope.1.take()
            {
                self.opt_offset = offset;
            }
            if name == "GLOBIGNORE" {
                self.shopt.glob.set_ignore(self.get(name).as_deref());
            }
            return;
        }
        self.variables.insert(
            name.into(),
            Variable {
                letter_case: None,
                temporary: false,
                dynamic: false,
                inherited: Default::default(),
                readonly: false,
                nameref: false,
                integer: false,
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

    pub fn unset(&mut self, args: &[String]) -> u8 {
        let mut index = 0;
        let mut literal_ref = false;
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
                    'n' => literal_ref = true,
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
        let mut status = 0;
        for original in &args[index..] {
            if literal_ref && !self.is_nameref(original) {
                continue;
            }
            let resolved;
            let name = if literal_ref || functions_only {
                original
            } else {
                match self.reference_name(original) {
                    Ok(Cow::Owned(n)) => {
                        resolved = n;
                        &resolved
                    }
                    Ok(_) => original,
                    Err(e) => {
                        eprintln!("tonari: unset: {original}: {e}");
                        status = 1;
                        continue;
                    }
                }
            };
            let base = array::subscript(name).map_or(name.as_str(), |(base, _)| base);
            if !functions_only
                && (Self::builtin_readonly(base)
                    || self.variables.get(base).is_some_and(|v| v.readonly))
            {
                eprintln!("tonari: unset: {base}: readonly variable");
                status = 1;
                continue;
            }
            let resolve_function = functions_only
                || !variables_only
                    && self
                        .declared_variable(name, false)
                        .is_none_or(|v| !v.declared);
            if resolve_function && self.readonly_functions.contains(name) {
                eprintln!("tonari: unset: {name}: readonly function");
                status = 1;
                continue;
            }
            if !functions_only {
                if let Some((base, index)) = array::subscript(name) {
                    if self.is_associative(base) {
                        match self.subscript_key(index) {
                            Ok(key) => self.remove_key(base, &key),
                            Err(e) => {
                                eprintln!("tonari: unset: {e}");
                                status = 1;
                            }
                        }
                        continue;
                    }
                    let variable = self.variables.get(base);
                    let array = variable.is_some_and(|v| v.elements.is_some());
                    if !array
                        && !variable
                            .map_or_else(|| self.inherited_value(base).is_some(), |v| v.declared)
                    {
                        continue;
                    }
                    let resolved = crate::arithmetic::expanded(index, self, 0)
                        .and_then(|n| self.relative_index(base, n));
                    match resolved {
                        Ok(i) if !array && i != 0 => {
                            eprintln!("tonari: unset: {base}: not an array variable");
                            status = 1;
                        }
                        Ok(i) => self.unset_element(base, i),
                        Err(e) => {
                            eprintln!("tonari: unset: {e}");
                            return 1;
                        }
                    }
                    continue;
                }
            }
            if resolve_function {
                self.forget_function(name);
            }
            if !functions_only && is_identifier(name) {
                self.unset_variable(name);
            }
        }
        status
    }

    fn logical_directory_ref(&self) -> io::Result<&Path> {
        if let Some(cwd) = self.logical_cwd.get() {
            return Ok(cwd);
        }
        if let Some(pwd) = std::env::var_os("PWD") {
            let path = Path::new(&pwd);
            if path.is_absolute()
                && !path.components().any(|c| c == Component::ParentDir)
                && same_directory(path, Path::new("."))
            {
                return Ok(self.logical_cwd.get_or_init(|| path.into()));
            }
        }
        let cwd = std::env::current_dir()?;
        Ok(self.logical_cwd.get_or_init(|| cwd))
    }

    pub fn logical_directory(&self) -> io::Result<PathBuf> {
        self.logical_directory_ref().map(Path::to_owned)
    }

    pub fn directory_builtin(&mut self, name: &str, args: &[String]) -> u8 {
        let mut physical = false;
        let mut require_pwd = false;
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
                    'e' if name == "cd" => require_pwd = true,
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
            if self.posix && physical {
                match self.try_change_directory(
                    Path::new(&target),
                    physical,
                    previous,
                    arg.is_some(),
                ) {
                    Ok(()) => return 0,
                    Err(DirectoryError::NoPath(e)) => {
                        if require_pwd {
                            eprintln!("tonari: cd: {e}");
                            return 1;
                        }
                        return 0;
                    }
                    Err(DirectoryError::Other(e)) => {
                        eprintln!("tonari: cd: {e}");
                        return if require_pwd { 2 } else { 1 };
                    }
                }
            }
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

    pub(crate) fn revisit_directory(&mut self, forward: bool) -> io::Result<bool> {
        let Some(visits) = &mut self.visits else {
            return Ok(false);
        };
        let mut index = visits.current;
        while let Some(next) = index.checked_add_signed(if forward { 1 } else { -1 }) {
            let Some(path) = visits.paths.get(next) else {
                break;
            };
            // Visits already contain normalized absolute paths. Resolve them once in the kernel.
            match std::env::set_current_dir(path) {
                Ok(()) => {
                    let cwd = path.clone();
                    Rc::make_mut(visits).current = next;
                    let old_pwd = self.get("PWD");
                    self.hashes.directory_changed();
                    self.finish_directory_change(cwd, old_pwd, false)?;
                    return Ok(true);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
                {
                    index = next
                }
                Err(e) => return Err(io::Error::new(e.kind(), format!("{}: {e}", path.display()))),
            }
        }
        Ok(false)
    }

    fn change_directory(
        &mut self,
        target: &Path,
        physical: bool,
        previous: bool,
        explicit: bool,
    ) -> io::Result<()> {
        self.try_change_directory(target, physical, previous, explicit)
            .map_err(|e| match e {
                DirectoryError::NoPath(e) | DirectoryError::Other(e) => e,
            })
    }

    fn try_change_directory(
        &mut self,
        target: &Path,
        physical: bool,
        previous: bool,
        explicit: bool,
    ) -> Result<(), DirectoryError> {
        let old_pwd = self.get("PWD");
        let base = self.logical_directory();
        let cdpath = if explicit
            && !previous
            && matches!(target.components().next(), Some(Component::Normal(_)))
        {
            self.get("CDPATH")
        } else {
            None
        };
        let mut paths = cdpath
            .iter()
            .flat_map(std::env::split_paths)
            .map(|dir| {
                let print = !dir.as_os_str().is_empty();
                (dir.join(target), print)
            })
            .chain(std::iter::once_with(|| (target.to_owned(), previous)));
        let mut variable = target
            .to_str()
            .filter(|_| explicit && !previous && self.shopt.cdable_vars);
        let mut error = io::Error::from(io::ErrorKind::NotFound);
        loop {
            let (candidate, print, via_variable) = if let Some((path, print)) = paths.next() {
                (path, print, false)
            } else if let Some(value) = variable.take().and_then(|name| self.get(name)) {
                (PathBuf::from(value), false, true)
            } else {
                break;
            };
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
                joined
                    .as_deref()
                    .and_then(|path| normalize_directory(path).ok())
            };
            // A renamed ancestor can invalidate the logical path while the
            // process still has a valid current directory for a relative chdir.
            let logical = logical.filter(|path| std::env::set_current_dir(path).is_ok());
            if logical.is_none()
                && let Err(e) = std::env::set_current_dir(&candidate)
            {
                error = e;
                continue;
            }
            self.hashes.directory_changed();
            self.logical_cwd.take();
            let cwd = match logical {
                Some(cwd) => cwd,
                None => std::env::current_dir().map_err(DirectoryError::NoPath)?,
            };
            if self.interactive && base.as_deref().ok() != Some(cwd.as_path()) {
                Rc::make_mut(self.visits.get_or_insert_with(Rc::default)).record(base.ok(), &cwd);
            }
            if via_variable {
                print_path(&candidate)?;
            }
            return self
                .finish_directory_change(cwd, old_pwd, print)
                .map_err(DirectoryError::Other);
        }
        Err(DirectoryError::Other(io::Error::new(
            error.kind(),
            format!("{}: {error}", target.display()),
        )))
    }

    fn finish_directory_change(
        &mut self,
        cwd: PathBuf,
        old_pwd: Option<String>,
        print: bool,
    ) -> io::Result<()> {
        self.logical_cwd = std::cell::OnceCell::from(cwd.clone());
        let old_readonly = self.readonly("OLDPWD");
        if !old_readonly {
            let old_export = self.is_exported("OLDPWD");
            self.variables.insert(
                "OLDPWD".into(),
                Variable {
                    letter_case: None,
                    temporary: false,
                    dynamic: false,
                    inherited: Default::default(),
                    readonly: false,
                    nameref: false,
                    integer: false,
                    array_assigned: false,
                    elements: None,
                    declared: true,
                    value: old_pwd,
                    exported: old_export,
                },
            );
        }
        let pwd_readonly = self.readonly("PWD");
        self.assign(
            "PWD",
            cwd.to_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 directory"))?,
        );
        if print {
            print_path(&cwd)?;
        }
        if old_readonly || pwd_readonly {
            return Err(io::Error::other(if old_readonly {
                "OLDPWD: readonly variable"
            } else {
                "PWD: readonly variable"
            }));
        }
        Ok(())
    }
}

impl Variable {
    fn attribute_flags(&self) -> impl Iterator<Item = u8> {
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
            (self.letter_case == Some(false), b'l'),
            (self.letter_case == Some(true), b'u'),
        ]
        .into_iter()
        .filter_map(|(on, flag)| on.then_some(flag))
    }
}

impl Session {
    pub(crate) fn array_declaration_fields(&self, name: &str) -> Vec<String> {
        let Some(v) = self.declared_variable(name, false) else {
            return Vec::new();
        };
        let mut assignment = name.as_bytes().to_vec();
        print_variable_value(&mut assignment, &v).expect("in-memory write");
        assignment.pop();
        vec![
            "declare".into(),
            format!(
                "-{}",
                v.attribute_flags().map(char::from).collect::<String>()
            ),
            String::from_utf8(assignment).unwrap(),
        ]
    }

    pub(crate) fn array_pair_fields(&self, name: &str, separate: bool) -> Vec<String> {
        let Some(var) = self.declared_variable(name, false) else {
            return Vec::new();
        };
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
        fields
    }

    pub(crate) fn attributes(&self, name: &str) -> Option<(String, bool)> {
        let resolved = self.reference_name(name).ok()?;
        let name = array::subscript(&resolved).map_or(resolved.as_ref(), |(base, _)| base);
        if let Some(var) = self.variables.get(name) {
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

fn print_declaration(out: &mut impl Write, name: &str, var: &Variable) -> io::Result<()> {
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

fn set_value(value: &str, posix: bool, locale: CharacterLocale) -> Cow<'_, str> {
    let ascii = matches!(locale, CharacterLocale::Ascii);
    if !posix
        && value
            .bytes()
            .any(|c| c.is_ascii_control() || ascii && !c.is_ascii())
    {
        return String::from_utf8(printf::ansi_quote(value.as_bytes(), ascii))
            .unwrap()
            .into();
    }
    if value
        .bytes()
        .any(|c| c.is_ascii_whitespace() || b"~!#$&*(){}[];<>?\\\"'|`".contains(&c))
    {
        crate::completion_spec::single_quote(value).into()
    } else {
        Cow::Borrowed(value)
    }
}

fn print_variable_value(out: &mut impl Write, var: &Variable) -> io::Result<()> {
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

fn print_value(out: &mut impl Write, value: &str) -> io::Result<()> {
    if value.bytes().any(|c| c.is_ascii_control()) {
        return out.write_all(&printf::ansi_quote(value.as_bytes(), false));
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
        session.enter_function(&mut Default::default());
        assert_eq!(
            session.declare("local", &[format!("{key}=local")], &[], 0),
            crate::Flow::Continue(0)
        );
        let prefix = session.bind(&[(key.into(), "temporary".into())]);
        assert_eq!(session.get(key).as_deref(), Some("temporary"));
        assert!(session.is_exported(key));
        session.restore(prefix);
        assert_eq!(session.get(key).as_deref(), Some("local"));
        session.leave_function();
        assert_eq!(session.get(key).as_deref(), Some("global"));
    }
}
