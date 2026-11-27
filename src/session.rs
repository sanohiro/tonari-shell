mod export_unset;
mod format;
use format::*;
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

macro_rules! bash_version {
    ($major:literal, $minor:literal, $patch:literal, $build:literal, $vendor:literal) => {
        pub(crate) const BASH_SERIES: &str = concat!($major, ".", $minor);
        pub(crate) const BASH_RELEASE: &str = concat!($major, ".", $minor, ".", $patch);
        const BASH_VERSION: &str =
            concat!($major, ".", $minor, ".", $patch, "(", $build, ")-", $vendor);
        const BASH_VERSINFO: [&str; 5] = [$major, $minor, $patch, $build, $vendor];
    };
}
bash_version!("5", "2", "0", "1", "tonari");

pub(crate) const READONLY: &str = "readonly variable";
pub(crate) const SUBSCRIPT_RANGE: &str = "array subscript out of range";

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use crate::parser::{FunctionDefinition, is_identifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum LetterCase {
    Lower,
    Upper,
    Capitalize,
}

#[derive(Clone, Default)]
struct Variable {
    // A binding keeps its name in saved scopes. The process environment is
    // immutable until exec or entry into a new shell session.
    inherited: std::cell::OnceCell<bool>,
    dynamic: bool,
    temporary: bool,
    letter_case: Option<LetterCase>,
    nameref: bool,
    readonly: bool,
    integer: bool,
    array_assigned: bool,
    elements: Option<Rc<array::Elements>>,
    declared: bool,
    value: Option<String>,
    exported: bool,
}

impl Variable {
    fn inherited(&self, name: &str) -> bool {
        *self
            .inherited
            .get_or_init(|| std::env::var_os(name).is_some())
    }

    fn scalar(value: Option<String>, exported: bool) -> Self {
        Self {
            declared: true,
            value,
            exported,
            ..Self::default()
        }
    }
}

// Any unrestricted mutation may change which bindings reach a child. Ordinary
// scalar assignments only change membership when allexport is enabled.
#[derive(Clone, Default)]
struct Variables {
    entries: BTreeMap<String, Variable>,
    environment_names: std::cell::OnceCell<Option<Rc<[String]>>>,
}

impl std::ops::Deref for Variables {
    type Target = BTreeMap<String, Variable>;
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl std::ops::DerefMut for Variables {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.environment_names.take();
        &mut self.entries
    }
}

impl Variables {
    fn value_mut(&mut self, name: &str, allexport: bool) -> Option<&mut Variable> {
        if allexport {
            self.environment_names.take();
        }
        self.entries.get_mut(name)
    }

    fn environment_entries(&self) -> impl Iterator<Item = (&String, &Variable)> {
        // Small or densely exported tables are cheaper to walk in key order.
        const MIN_BINDINGS: usize = 32;
        let selected = if self.entries.len() < MIN_BINDINGS {
            None
        } else {
            self.environment_names
                .get_or_init(|| {
                    let max_names = self.entries.len() / 4;
                    let names: Vec<_> = self
                        .entries
                        .iter()
                        .filter(|(name, var)| var.exported || var.inherited(name))
                        .take(max_names + 1)
                        .map(|(name, _)| name.clone())
                        .collect();
                    (names.len() <= max_names).then(|| Rc::from(names))
                })
                .as_deref()
        };
        let mut names = selected.into_iter().flatten();
        let mut all = selected.is_none().then(|| self.entries.iter());
        std::iter::from_fn(move || {
            if let Some(all) = &mut all {
                all.next()
            } else {
                names
                    .next()
                    .and_then(|name| self.entries.get_key_value(name))
            }
        })
    }
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
    variables: Variables,
    inherited_names: std::cell::OnceCell<Rc<Vec<String>>>,
    dynamic: dynamic::Dynamic,
    groups_unset: bool,
    funcname_unset: bool,
    word_breaks_unset: bool,
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
    pub physical: bool,
    pub keyword: bool,
    pub logout_started: bool,
    pub allexport: bool,
    pub notify: bool,
    pub noexec: bool,
    pub ignoreeof: bool,
    pub ignoreeof_limit: usize,
    pub xtrace: bool,
    pub onecmd: bool,
    pub subshell_depth: u32,
    pub histexpand: bool,
    pub emacs: bool,
    pub vi: bool,
    pub history_enabled: bool,
    pub verbose: bool,
    pub(crate) history: Option<Rc<crate::history::Entries>>,
    pub(crate) history_expansion: Option<Box<crate::history::expand::Memory>>,
    pub(crate) initial_history: Option<Rc<crate::history::InitialJournal>>,
    pub command_number: usize,
    pub(crate) evaluating: bool,
    pub(crate) trace_depth: u32,
    pub(crate) signal_line: Option<usize>,
    pub command_string: bool,
    pub(crate) execution_string: Option<Rc<String>>,
    pub(crate) source_set_args: bool,
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
    pub stdin_replaced: bool,
    pub noglob: bool,
    pub noclobber: bool,
    pub shopt: options::ShellOptions,
    pub(crate) external_env: std::cell::RefCell<Option<Rc<crate::execute::Environment>>>,
    pub(crate) eval_cache: crate::input::EvalCache,
    pub regex: [[Option<Rc<crate::test_expression::Compiled>>; 4]; 2],
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
    pub(crate) substitution_scopes: u32,
    pub function_names: Vec<String>,
    pub source_names: Vec<Rc<str>>,
    pub current_command: Option<crate::parser::Location>,
    pub debug_args: Vec<Vec<String>>,
    pub debug_base: std::cell::OnceCell<Vec<String>>,
    pub traced_functions: std::collections::BTreeSet<String>,
    pub call_lines: Vec<usize>,
    pub function_environment: BTreeMap<String, Option<String>>,
    pub loop_depth: u32,
    pub parent_loop_in_subshell: bool,
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
pub struct Bindings {
    variables: BTreeMap<String, Option<Variable>>,
    opt_offset: Option<usize>,
    temporary: bool,
    set_options: Option<std::num::NonZeroU32>,
    posix_settings: Option<PosixSettings>,
}

#[derive(Clone, Copy)]
struct PosixSettings {
    expand_aliases: bool,
    inherit_errexit: bool,
    interactive_comments: bool,
    shift_verbose: bool,
    sourcepath: bool,
}

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
    /// Fork a copy that can run shell code. Reset inherited traps and waitable
    /// process substitutions only in the child. The caller configures job control
    /// and descriptors before running the command and exiting the child.
    pub(crate) fn fork_shell(&mut self) -> io::Result<libc::pid_t> {
        match unsafe { libc::fork() } {
            -1 => Err(io::Error::last_os_error()),
            0 => {
                self.traps.after_fork(self.errtrace, self.functrace);
                self.substitutions.after_fork();
                Ok(0)
            }
            pid => Ok(pid),
        }
    }

    fn convert_case(&self, mut text: String, letter_case: Option<LetterCase>) -> String {
        let Some(mode) = letter_case else { return text };
        if mode == LetterCase::Capitalize {
            return crate::pattern::capitalize(text, self.character_locale());
        }
        let upper = mode == LetterCase::Upper;
        if text.is_ascii() && !text.as_bytes().contains(&if upper { b'i' } else { b'I' }) {
            if upper {
                text.make_ascii_uppercase();
            } else {
                text.make_ascii_lowercase();
            }
            text
        } else {
            crate::pattern::case_chars(text, upper, true, None, self.character_locale())
        }
    }

    pub(crate) fn locale_category(&self, category: &str) -> Option<Cow<'_, str>> {
        ["LC_ALL", category, "LANG"]
            .into_iter()
            .find_map(|key| self.scalar_setting(key).filter(|s| !s.is_empty()))
    }

    pub(crate) fn ascii_quoting(&self) -> bool {
        self.locale_category("LC_CTYPE")
            .is_some_and(|name| matches!(name.as_ref(), "C" | "POSIX"))
    }

    pub(crate) fn pattern_locale_overridden(&self) -> bool {
        ["LANG", "LC_ALL", "LC_COLLATE", "LC_CTYPE"]
            .iter()
            .any(|key| self.variables.contains_key(*key))
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
        self.visible_names(prefix)
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn inherited_variable_names(&self) -> &[String] {
        // Only a fresh shell session changes the inherited process environment.
        // Shell bindings, tombstones and optional names remain live overlays.
        self.inherited_names.get_or_init(|| {
            let mut names: Vec<_> = std::env::vars_os()
                .filter_map(|(key, _)| key.into_string().ok())
                .filter(|key| is_identifier(key))
                .collect();
            names.extend(
                [
                    "PWD",
                    "SHELL",
                    "TERM",
                    "_",
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
                    "BASHPID",
                    "BASH_ARGV0",
                    "BASH_COMMAND",
                    "BASH_SUBSHELL",
                    "LINENO",
                    "EPOCHSECONDS",
                    "EPOCHREALTIME",
                    "BASH_VERSINFO",
                    "BASH_ALIASES",
                    "BASH_CMDS",
                    "SECONDS",
                    "RANDOM",
                    "SRANDOM",
                    "BASH_MONOSECONDS",
                    "BASH_ARGC",
                    "BASH_ARGV",
                    "BASH_SOURCE",
                    "BASH_LINENO",
                    "FUNCNAME",
                    "DIRSTACK",
                    "PIPESTATUS",
                    "HISTCMD",
                    "GROUPS",
                    "HOSTTYPE",
                    "HOSTNAME",
                    "PS4",
                    "OSTYPE",
                    "MACHTYPE",
                ]
                .into_iter()
                .map(str::to_owned),
            );
            names.sort_unstable();
            names.dedup();
            Rc::new(names)
        })
    }

    fn visible_names(&self, prefix: &str) -> Vec<&str> {
        let base = self.inherited_variable_names();
        let start = base.partition_point(|key| key.as_str() < prefix);
        let mut names: Vec<_> = base[start..]
            .iter()
            .take_while(|key| key.starts_with(prefix))
            .map(String::as_str)
            .filter(|key| {
                !(self.funcname_unset && *key == "FUNCNAME" || self.variables.contains_key(*key))
            })
            .collect();
        for (key, active) in [
            ("COMP_WORDBREAKS", !self.word_breaks_unset),
            ("BASH_EXECUTION_STRING", self.execution_string.is_some()),
        ] {
            if active && key.starts_with(prefix) && !self.variables.contains_key(key) {
                names.push(key);
            }
        }
        use std::ops::Bound::{Included, Unbounded};
        for (key, var) in self
            .variables
            .range::<str, _>((Included(prefix), Unbounded))
        {
            if !key.starts_with(prefix) {
                break;
            }
            if var.declared {
                names.push(key);
            }
        }
        names.sort_unstable();
        names.dedup();
        names
    }

    pub fn shell_pid(&self) -> u32 {
        self.shell_pid.0
    }

    pub fn printf(&mut self, args: &[String]) -> u8 {
        printf::run(self, args, None)
    }

    pub fn read(&mut self, args: &[String], here: Option<(i32, &[u8])>, status: u8) -> crate::Flow {
        read::run(self, args, here, status)
    }

    pub fn shift(&mut self, args: &[String]) -> u8 {
        let args = match args.split_first() {
            Some((flag, rest)) if flag == "--" => rest,
            _ => args,
        };
        if args.len() > 1 {
            diagnostic!("shift: too many arguments");
            return 1;
        }
        let count = args.first().map_or(Ok(1), |s| s.trim().parse::<i64>());
        let Ok(count) = count else {
            diagnostic!("shift: numeric argument required");
            return 1;
        };
        if count < 0 || count as u64 > self.positional.len() as u64 {
            if self.shopt.shift_verbose {
                diagnostic!("shift: shift count out of range");
            }
            return 1;
        }
        if count > 0 {
            self.positional.nth(count as usize - 1);
        }
        0
    }

    pub fn is_nameref(&self, name: &str) -> bool {
        self.has_namerefs && self.variables.get(name).is_some_and(|v| v.nameref)
    }

    pub(crate) fn valid_reference_target(name: &str) -> bool {
        is_identifier(name)
            || array::subscript(name).is_some_and(|(base, index)| {
                is_identifier(base)
                    && !index.is_empty()
                    && (index.bytes().all(crate::word::is_name_char)
                        || crate::parser::index_end(&name[base.len()..]).ok()
                            == Some(name.len() - base.len()))
            })
    }

    pub(crate) fn assign_for_reference(
        &mut self,
        name: &str,
        value: String,
    ) -> Result<(), &'static str> {
        if !Self::valid_reference_target(&value) {
            return Err("invalid name reference");
        }
        // A for-loop retargets the last reference, even if its target is readonly.
        let mut current = name;
        for _ in 0..32 {
            if let Some(next) = self
                .variables
                .get(current)
                .and_then(|v| v.value.as_deref())
                .filter(|next| self.is_nameref(next))
            {
                current = next;
                continue;
            }
            let key = if current == name {
                Cow::Borrowed(name)
            } else {
                Cow::Owned(current.to_owned())
            };
            let var = self
                .variables
                .get_mut(key.as_ref())
                .expect("existing name reference");
            if var.readonly {
                return Err(crate::session::READONLY);
            }
            var.value = Some(value);
            var.declared = true;
            return Ok(());
        }
        Err("circular name reference")
    }

    pub fn reference_name<'a>(&self, name: &'a str) -> Result<Cow<'a, str>, &'static str> {
        self.scoped_reference_name(name, false)
    }

    fn scoped_reference_name<'a>(
        &self,
        name: &'a str,
        global: bool,
    ) -> Result<Cow<'a, str>, &'static str> {
        if !self.has_namerefs {
            return Ok(Cow::Borrowed(name));
        }
        let split = |s: &str| {
            array::subscript(s)
                .map(|(base, _)| base.len())
                .unwrap_or(s.len())
        };
        let (mut base, mut suffix) = name.split_at(split(name));
        for depth in 0..32 {
            let target = self
                .scoped_variable(base, global)
                .filter(|v| v.nameref)
                .and_then(|v| v.value.as_deref())
                .filter(|s| !s.is_empty());
            let Some(target) = target else {
                return Ok(if depth == 0 {
                    Cow::Borrowed(name)
                } else if suffix.is_empty() {
                    Cow::Owned(base.to_owned())
                } else {
                    Cow::Owned(format!("{base}{suffix}"))
                });
            };
            let (next, index) = target.split_at(split(target));
            if !index.is_empty() {
                if !suffix.is_empty() {
                    return Err("invalid indexed name reference");
                }
                suffix = index;
            }
            base = next;
        }
        Err("circular name reference")
    }

    // Ordinary shell-authored scalars can be inspected without copying their value.
    pub(crate) fn scalar_setting(&self, name: &str) -> Option<Cow<'_, str>> {
        match self.variables.get(name) {
            Some(var) if !var.nameref && !var.dynamic && var.elements.is_none() => {
                return var.value.as_deref().map(Cow::Borrowed);
            }
            None => {
                // In-process script reentry changes only PATH and directory/level
                // variables. These inherited history settings stay constant.
                static HISTORY: [std::sync::OnceLock<Option<String>>; 4] =
                    [const { std::sync::OnceLock::new() }; 4];
                let slot = match name {
                    "HISTSIZE" => Some(0),
                    "HISTCONTROL" => Some(1),
                    "HISTIGNORE" => Some(2),
                    "histchars" => Some(3),
                    _ => None,
                };
                if let Some(i) = slot {
                    return HISTORY[i]
                        .get_or_init(|| std::env::var(name).ok())
                        .as_deref()
                        .map(Cow::Borrowed);
                }
            }
            Some(_) => {}
        }
        self.get(name).map(Cow::Owned)
    }

    pub fn get(&self, name: &str) -> Option<String> {
        if name == "_" {
            return self
                .last_argument
                .clone()
                .or_else(|| std::env::var("_").ok())
                .or_else(|| std::env::args_os().next()?.into_string().ok());
        }
        if let Some(base) = name.strip_prefix(crate::word::ARRAY_INDEXES_PREFIX) {
            let resolved = self.reference_name(base).ok()?;
            let base = resolved.as_ref();
            let snapshot;
            let map = if self.live_table(base) {
                snapshot = self.table_snapshot(base);
                Some(&snapshot)
            } else {
                self.variables.get(base).and_then(Variable::associative)
            };
            if let Some(map) = map {
                return Some(
                    map.ordered()
                        .into_iter()
                        .map(|(key, _)| key)
                        .collect::<Vec<_>>()
                        .join("\0"),
                );
            }
            let mut text = String::new();
            let mut decimal = itoa::Buffer::new();
            for index in self.indices(base) {
                if !text.is_empty() {
                    text.push('\0');
                }
                text.push_str(decimal.format(index));
            }
            return Some(text);
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
                    if error == crate::session::SUBSCRIPT_RANGE {
                        diagnostic!("{base}: bad array subscript");
                    }
                })
                .ok()?;
            return self.element(base, i);
        }
        if self.live_table(name) {
            return self.assoc_value(name, "0").map(str::to_owned);
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
            "BASH_EXECUTION_STRING" if self.execution_string.is_some() => {
                self.execution_string.as_deref().cloned()
            }
            "HISTCMD" | "BASH_MONOSECONDS" | "RANDOM" | "SECONDS" | "SRANDOM" => {
                self.dynamic_number(name).map(|v| v.to_string())
            }
            "COMP_WORDBREAKS" => Some(self.observe_text(
                "COMP_WORDBREAKS",
                crate::complete::default_word_breaks(self.shopt.hostcomplete).into(),
            )),
            "BASHPID" => {
                let pid = std::process::id();
                self.dynamic.pid.set(Some(pid));
                Some(pid.to_string())
            }
            "BASH_SUBSHELL" => {
                self.dynamic.depth.set(Some(self.subshell_depth));
                Some(self.subshell_depth.to_string())
            }
            "BASH_ARGV0" => Some(self.observe_text("BASH_ARGV0", self.arg0.clone())),
            "EPOCHSECONDS" | "EPOCHREALTIME" => {
                let dt = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?;
                Some(if name == "EPOCHSECONDS" {
                    self.observe_text("EPOCHSECONDS", dt.as_secs().to_string())
                } else {
                    self.observe_text(
                        "EPOCHREALTIME",
                        format!("{}.{:06}", dt.as_secs(), dt.subsec_micros()),
                    )
                })
            }
            "LINENO" => {
                let n = self.line();
                self.dynamic.lineno.set(Some(n));
                Some(n.to_string())
            }
            "BASH_COMMAND" => self
                .current_command
                .as_ref()
                .map(|c| self.observe_text("BASH_COMMAND", c.text().to_owned())),
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
            "BASH_VERSION" => Some(BASH_VERSION.into()),
            "GROUPS" | "BASH_VERSINFO" => self.element(name, 0),
            "SHELL" => std::env::var(name).ok().or_else(crate::word::account_shell),
            "TERM" => Some(std::env::var(name).unwrap_or_else(|_| "dumb".into())),
            "HOSTNAME" => std::env::var(name)
                .ok()
                .or_else(|| hostname().map(str::to_owned)),
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
            "IFS" => Some(crate::word::DEFAULT_IFS.into()),
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

    pub fn initialize_shell_level(&mut self, env: &crate::startup::Environment) {
        if !env.pwd
            && let Some(directory) = self.get("PWD")
        {
            self.assign("PWD", &directory);
        }
        let level = env.shell_level.saturating_add(1);
        let level = if level >= 1000 {
            diagnostic!("warning: shell level ({level}) too high, resetting to 1");
            1
        } else {
            level.max(0)
        };
        self.variables.insert(
            "SHLVL".into(),
            Variable::scalar(Some(level.to_string()), true),
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
                | "IGNOREEOF"
                | "ignoreeof"
                | "POSIXLY_CORRECT"
                | "BASH_COMPAT"
                | "HISTSIZE"
                | "HISTFILESIZE"
                | "OPTIND"
                | "PATH"
                | "PIPESTATUS"
        )
    }

    fn plain_read_target(&self, name: &str) -> bool {
        is_identifier(name)
            && Self::ordinary_assignment(name)
            && !Self::builtin_readonly(name)
            && self
                .variables
                .get(name)
                .is_none_or(|v| !v.nameref && !v.integer && !v.dynamic && v.elements.is_none())
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
        let Some(var) = self.variables.value_mut(name, self.allexport) else {
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

    fn apply_variable_effects(&mut self, name: &str, value: &str) {
        match name {
            "BASH_COMPAT" => self.apply_compat(value),
            "POSIXLY_CORRECT" => self.apply_posix_mode(true),
            "OPTIND" => self.opt_offset = 0,
            "IGNOREEOF" | "ignoreeof" => self.set_eof_variable(Some(value)),
            "GLOBIGNORE" => self.shopt.glob.set_ignore(Some(value)),
            "PATH" => self.hashes.clear(),
            _ => {}
        }
    }

    pub(crate) fn assign_value(&mut self, name: &str, mut value: Cow<'_, str>) {
        match self.reference_name(name) {
            Ok(Cow::Owned(target)) => {
                if let Err(e) = self.assign_target(&target, &value) {
                    diagnostic!("{name}: {e}");
                }
                return;
            }
            Err(e) => {
                diagnostic!("{name}: {e}");
                return;
            }
            _ => {}
        }
        if self.is_nameref(name) && !Self::valid_reference_target(&value) {
            diagnostic!("{name}: invalid name reference");
            return;
        }
        if self.readonly(name) {
            return;
        }
        if self.live_table(name) {
            let _ = self.assign_key(name, "0".into(), value.into_owned());
            return;
        }
        if self.protected_array(name) {
            return;
        }
        self.apply_variable_effects(name, &value);
        if matches!(name, "HISTSIZE" | "HISTFILESIZE") {
            self.apply_history_limit(name, &value);
        }
        if name == "BASH_ARGV0" {
            self.arg0 = value.to_string();
        }
        if name == "OPTIND" {
            value = Cow::Owned(value.trim().parse::<i64>().unwrap_or(0).to_string());
        }
        let live = dynamic::is_dynamic(name) && self.variables.get(name).is_none_or(|v| v.dynamic);
        if let Some(upper) = self.variables.get(name).and_then(|v| v.letter_case) {
            value = Cow::Owned(self.convert_case(value.into_owned(), Some(upper)));
        }
        if live {
            self.set_dynamic(name, &value);
        }
        if let Some(var) = self.variables.value_mut(name, self.allexport) {
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
                dynamic: live,
                integer: live && dynamic::integer_attribute(name),
                declared: true,
                value: Some(value.into_owned()),
                exported,
                ..Variable::default()
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
        let var = self.variables.value_mut(name, self.allexport)?;
        if var.dynamic || var.nameref || var.elements.is_some() {
            return None;
        }
        if var.readonly {
            return Some(Err(crate::session::READONLY));
        }
        let text = var.value.as_mut()?;
        itoa::Buffer::new().format(value).clone_into(text);
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
        if !Self::ordinary_assignment(name) || Self::builtin_readonly(name) {
            return false;
        }
        let Some(var) = self.variables.value_mut(name, self.allexport) else {
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
            itoa::Buffer::new().format(result).clone_into(current);
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

    pub(crate) fn completion_breaks(&self) -> Cow<'_, str> {
        let default = crate::complete::default_word_breaks(self.shopt.hostcomplete);
        if self.word_breaks_unset || !self.variables.contains_key("COMP_WORDBREAKS") {
            return Cow::Borrowed(default);
        }
        if let Some(value) = self.stored_scalar("COMP_WORDBREAKS") {
            return Cow::Borrowed(value);
        }
        self.get("COMP_WORDBREAKS")
            .map_or(Cow::Borrowed(default), Cow::Owned)
    }

    pub(crate) fn stored_scalar(&self, name: &str) -> Option<&str> {
        if name == "_" {
            return self.last_argument.as_deref();
        }
        if matches!(name, "BASHPID" | "BASHOPTS" | "SHELLOPTS" | "PIPESTATUS") {
            return None;
        }
        let Some(var) = self.variables.get(name) else {
            return (name == "BASH_EXECUTION_STRING")
                .then(|| self.execution_string.as_deref().map(String::as_str))
                .flatten();
        };
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

    pub(crate) fn assignment_attributes(&self, name: &str) -> (bool, bool) {
        self.variables.get(name).map_or_else(
            || (dynamic::integer_attribute(name), false),
            |v| (v.integer, v.letter_case.is_some()),
        )
    }

    pub fn integer(&self, name: &str) -> bool {
        self.variables
            .get(name)
            .map_or_else(|| dynamic::integer_attribute(name), |v| v.integer)
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

    pub(crate) fn with_trap_signal(
        &mut self,
        number: i32,
        run: impl FnOnce(&mut Self) -> crate::Flow,
    ) -> crate::Flow {
        const NAME: &str = "BASH_TRAPSIG";
        let saved = self.variables.get(NAME).cloned();
        self.assign_value(NAME, Cow::Owned(number.to_string()));
        let result = run(self);
        if let Some(var) = saved {
            self.variables.insert(NAME.into(), var);
        } else {
            self.variables.remove(NAME);
        }
        result
    }

    pub fn save_variables(&self, names: &[&str]) -> Bindings {
        Bindings {
            variables: names
                .iter()
                .map(|&name| (name.into(), self.variables.get(name).cloned()))
                .collect(),
            opt_offset: names.contains(&"OPTIND").then_some(self.opt_offset),
            ..Bindings::default()
        }
    }

    // Special builtins retain their exported prefix bindings in POSIX mode.
    // Move the prepared strings and avoid snapshotting variables for restoration.
    pub fn persist_prefix(&mut self, assignments: &mut Vec<(String, String)>) {
        for (name, value) in assignments.drain(..) {
            // A special builtin's environment persists in the nearest real
            // binding. A function prefix can itself hide another prefix.
            if self.variables.get(&name).is_some_and(|v| v.temporary) {
                for scope in self.locals.iter_mut().rev() {
                    if let Some(previous) = scope.variables.remove(&name)
                        && !previous.as_ref().is_some_and(|v| v.temporary)
                    {
                        break;
                    }
                }
            }
            self.apply_variable_effects(&name, &value);
            let var = self.variables.entry(name).or_default();
            var.exported = true;
            var.declared = true;
            var.temporary = false;
            if let Some(array) = var.elements.as_mut() {
                Rc::make_mut(array).insert(0, value);
            } else {
                var.value = Some(value);
            }
        }
    }

    pub fn bind(&mut self, assignments: &[(String, String)]) -> Bindings {
        let mut saved = Bindings::default();
        for (name, value) in assignments {
            if self.readonly(name) {
                continue;
            }
            if name == "POSIXLY_CORRECT" && saved.posix_settings.is_none() {
                saved.posix_settings = Some(PosixSettings {
                    expand_aliases: self.shopt.expand_aliases,
                    inherit_errexit: self.shopt.inherit_errexit,
                    interactive_comments: self.shopt.interactive_comments,
                    shift_verbose: self.shopt.shift_verbose,
                    sourcepath: self.shopt.sourcepath,
                });
            }
            if name == "OPTIND" {
                saved.opt_offset.get_or_insert(self.opt_offset);
            }
            self.apply_variable_effects(name, value);
            saved
                .variables
                .entry(name.clone())
                .or_insert_with(|| self.variables.get(name).cloned());
            self.variables.insert(
                name.clone(),
                Variable {
                    temporary: true,
                    declared: true,
                    value: Some(value.clone()),
                    exported: true,
                    ..Variable::default()
                },
            );
        }
        saved
    }

    pub fn restore(&mut self, bindings: Bindings) {
        if let Some(offset) = bindings.opt_offset {
            self.opt_offset = offset;
        }
        for (name, original) in bindings.variables {
            if name == "BASH_COMPAT" {
                let inherited;
                let value = if let Some(v) = &original {
                    v.value.as_deref()
                } else {
                    inherited = std::env::var("BASH_COMPAT").ok();
                    inherited.as_deref()
                };
                self.apply_compat(value.unwrap_or(""));
            }
            if name == "POSIXLY_CORRECT" {
                let enabled = original.as_ref().map_or_else(
                    || std::env::var_os("POSIXLY_CORRECT").is_some(),
                    |v| v.value.is_some(),
                );
                self.apply_posix_mode(enabled);
                if let Some(old) = bindings.posix_settings {
                    self.shopt.expand_aliases = old.expand_aliases;
                    self.shopt.inherit_errexit = old.inherit_errexit;
                    self.shopt.interactive_comments = old.interactive_comments;
                    self.shopt.shift_verbose = old.shift_verbose;
                    self.shopt.sourcepath = old.sourcepath;
                }
            }
            if matches!(name.as_str(), "IGNOREEOF" | "ignoreeof") {
                let inherited;
                let previous = if let Some(var) = &original {
                    var.value.as_deref()
                } else {
                    inherited = std::env::var(&name).ok();
                    inherited.as_deref()
                };
                self.set_eof_variable(previous);
            }
            if name == "GLOBIGNORE" {
                let inherited;
                let value = if let Some(var) = &original {
                    var.value.as_deref()
                } else {
                    inherited = std::env::var("GLOBIGNORE").ok();
                    inherited.as_deref()
                };
                self.shopt.glob.set_ignore(value);
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
            .environment_entries()
            .filter_map(|(name, var)| {
                if var.exported && var.elements.is_none() && var.value.is_some() {
                    let value = match name.as_str() {
                        "BASHOPTS" | "SHELLOPTS" => {
                            Some(Cow::Owned(self.option_list(name == "SHELLOPTS")))
                        }
                        _ => var.value.as_deref().map(Cow::Borrowed),
                    };
                    Some((name.as_str(), value))
                } else if var.inherited(name) {
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
            .rposition(|scope| !scope.temporary || scope.variables.contains_key(name))
    }

    pub fn enter_scope(&mut self, bindings: &mut Bindings, temporary: bool) {
        let mut scope = std::mem::take(bindings);
        scope.temporary = temporary;
        self.locals.push(scope);
    }

    pub fn leave_scope(&mut self) {
        let scope = self.locals.pop().expect("variable scope");
        let saved_options = scope.set_options;
        self.restore(scope);
        if let Some(bits) = saved_options {
            self.restore_set_options(bits.get() & !(1 << 31));
        }
    }

    pub fn enter_function(&mut self, bindings: &mut Bindings) {
        self.enter_scope(bindings, false);
        self.function_depth += 1;
    }

    pub(crate) fn local_reply(&mut self, last: u8) {
        if self.shopt.localvar_inherit
            || self.variables.contains_key("REPLY")
            || std::env::var_os("REPLY").is_some()
        {
            self.declare(
                "local",
                &["REPLY".into()],
                &[],
                last,
                &mut Bindings::default(),
            );
        } else {
            self.locals
                .iter_mut()
                .rev()
                .find(|scope| !scope.temporary)
                .expect("function scope")
                .variables
                .insert("REPLY".into(), None);
            self.variables.insert("REPLY".into(), Variable::default());
        }
    }

    pub fn leave_function(&mut self) {
        self.leave_scope();
        self.function_depth -= 1;
    }
}

pub(crate) fn hostname() -> Option<&'static str> {
    static NAME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let mut bytes = [0u8; 256];
        if unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len() - 1) } != 0 {
            return None;
        }
        let end = memchr::memchr(0, &bytes).unwrap_or(bytes.len());
        Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
    })
    .as_deref()
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
            session.declare(
                "local",
                &[format!("{key}=local")],
                &[],
                0,
                &mut Default::default()
            ),
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
