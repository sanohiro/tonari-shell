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

use foldhash::HashMap;
use std::borrow::Cow;
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum BindingOrigin {
    #[default]
    Ordinary,
    CommandPrefix,
    LocalFromPrefix,
}

#[derive(Clone, Default)]
struct Variable {
    // A binding keeps its name in saved scopes. The process environment is
    // immutable until exec or entry into a new shell session.
    inherited: std::cell::OnceCell<bool>,
    dynamic: bool,
    origin: BindingOrigin,
    letter_case: Option<LetterCase>,
    nameref: bool,
    readonly: bool,
    traced: bool,
    integer: bool,
    array_assigned: bool,
    elements: Option<Rc<array::Elements>>,
    declared: bool,
    value: Option<String>,
    char_count: std::cell::Cell<u32>,
    exported: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NameList {
    Declared,
    Assigned,
}

impl Variable {
    fn visible_in(&self, list: NameList) -> bool {
        self.declared
            && (list == NameList::Declared
                || self.value.is_some()
                || self.array_assigned
                || self.dynamic)
    }

    fn scalar_mut(&mut self) -> &mut Option<String> {
        self.char_count.set(0);
        &mut self.value
    }

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
    entries: HashMap<String, Variable>,
    environment_names: std::cell::OnceCell<Option<Rc<[String]>>>,
    prefix_names: std::cell::RefCell<Option<(String, Vec<String>)>>,
}

impl std::ops::Deref for Variables {
    type Target = HashMap<String, Variable>;
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl std::ops::DerefMut for Variables {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.environment_names.take();
        *self.prefix_names.get_mut() = None;
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

    fn append_visible<'a>(&'a self, prefix: &str, listing: NameList, names: &mut Vec<&'a str>) {
        if prefix.is_empty() || self.entries.len() < 32 {
            names.extend(self.entries.iter().filter_map(|(key, var)| {
                (var.visible_in(listing) && key.starts_with(prefix)).then_some(key.as_str())
            }));
            return;
        }
        // Typing another character narrows the preceding match set. Keep keys
        // for just that prefix and check declaration visibility on every query.
        let mut cache = self.prefix_names.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|(old, _)| !prefix.starts_with(old.as_str()))
        {
            let keys = self
                .entries
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect();
            *cache = Some((prefix.to_owned(), keys));
        }
        let (old, keys) = cache.as_mut().unwrap();
        if old.len() < prefix.len() {
            keys.retain(|key| key.starts_with(prefix));
            prefix.clone_into(old);
        }
        for key in keys.iter() {
            if let Some((key, var)) = self.entries.get_key_value(key)
                && var.visible_in(listing)
            {
                names.push(key.as_str());
            }
        }
    }

    fn environment_entries(&self) -> impl Iterator<Item = (&String, &Variable)> {
        // Small or densely exported tables are cheaper to walk directly.
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
    listed_environment: std::cell::OnceCell<Rc<HashMap<OsString, OsString>>>,
    inherited_path: std::cell::OnceCell<OsString>,
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
    pub privileged: bool,
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
    pub regex: [Vec<Rc<crate::test_expression::Compiled>>; 2],
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
    default_function_source: Option<Rc<str>>,
    pub current_command: Option<crate::parser::Location>,
    pub debug_args: Vec<Vec<String>>,
    pub debug_base: std::cell::OnceCell<Vec<String>>,
    pub traced_functions: std::collections::BTreeSet<String>,
    pub call_lines: Vec<usize>,
    function_environment: HashMap<String, functions::Export>,
    pub loop_depth: u32,
    pub parent_loop_in_subshell: bool,
    pub editor_kill: crate::editor::KillRing,
    pub editor_queue: std::collections::VecDeque<u8>,
    pub bindings: crate::editor::bindings::Bindings,
    pub disabled_builtins: std::collections::BTreeSet<String>,
    pub aliases: crate::alias::Aliases,
    pub completions: crate::completion_spec::Registry,
    pub readonly_functions: std::collections::BTreeSet<String>,
    pub(crate) functions: functions::FunctionTable,
    locals: Vec<Bindings>,
}

#[derive(Clone, Default)]
pub struct Bindings {
    variables: HashMap<String, Option<Variable>>,
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

pub(crate) enum MemoryInput<'a> {
    Read(read::Options<'a>, &'a str, u8),
    Mapfile(mapfile::Options<'a>, &'a [u8]),
}

impl MemoryInput<'_> {
    pub(crate) fn run(self, session: &mut Session, name: &str, last: u8) -> crate::Flow {
        match self {
            Self::Read(opts, line, status) => {
                read::assign_fields(session, &opts, line, &[], status)
            }
            Self::Mapfile(opts, bytes) => {
                mapfile::run_parsed(session, name, opts, last, Some((0, bytes)))
            }
        }
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

    pub(crate) fn tilde_home(&self) -> Option<Cow<'_, str>> {
        self.scalar_setting("HOME")
            .or_else(|| crate::word::account_home().map(Cow::Owned))
    }

    /// Preserve the slash from ~/ even when HOME is empty. Additional
    /// slashes in the suffix are still relative to the expanded home.
    pub(crate) fn tilde_path(&self, suffix: &str) -> Option<PathBuf> {
        let home = self.tilde_home()?;
        let parent = if home.is_empty() { "/" } else { home.as_ref() };
        Some(Path::new(parent).join(suffix.trim_start_matches('/')))
    }

    /// A personal configuration path must never become relative through an
    /// explicitly empty HOME. An unset HOME uses the account directory.
    pub(crate) fn personal_home(&self) -> Option<Cow<'_, str>> {
        self.tilde_home().filter(|h| !h.is_empty())
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
            .find_map(|name| self.scalar_setting(name).filter(|s| !s.is_empty()));
        CharacterLocale::from_name(name.as_deref().unwrap_or(""))
    }

    pub fn variable_names(&self, prefix: &str) -> Vec<String> {
        self.visible_names(prefix, NameList::Assigned)
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    // Bulk variable output repeatedly looks up inherited values. The process
    // environment is immutable for this session and shell bindings overlay it.
    fn listed_environment(&self) -> &HashMap<OsString, OsString> {
        self.listed_environment
            .get_or_init(|| Rc::new(std::env::vars_os().collect()))
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
                    "OLDPWD",
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

    fn visible_names(&self, prefix: &str, listing: NameList) -> Vec<&str> {
        let base = self.inherited_variable_names();
        let start = base.partition_point(|key| key.as_str() < prefix);
        let mut names: Vec<_> = base[start..]
            .iter()
            .take_while(|key| key.starts_with(prefix))
            .map(String::as_str)
            .filter(|key| {
                !(self.variables.contains_key(*key)
                    || *key == "FUNCNAME"
                        && (self.funcname_unset
                            || listing == NameList::Assigned && self.function_depth == 0)
                    || *key == "OLDPWD"
                        && listing == NameList::Assigned
                        && std::env::var_os("OLDPWD").is_none()
                    || *key == "PIPESTATUS"
                        && listing == NameList::Assigned
                        && self.pipe_status.first.is_none())
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
        self.variables.append_visible(prefix, listing, &mut names);
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

    fn memory_array_target(&self, name: &str) -> bool {
        is_identifier(name)
            && Self::ordinary_assignment(name)
            && !self.protected_array(name)
            && self.variables.get(name).is_none_or(|v| {
                !v.readonly && !v.integer && !v.nameref && !v.dynamic && v.associative().is_none()
            })
    }

    pub(crate) fn memory_input<'a>(
        &self,
        read: bool,
        args: &'a [String],
        bytes: &'a [u8],
    ) -> Option<MemoryInput<'a>> {
        if read {
            read::prepare_memory(self, args, bytes)
        } else {
            mapfile::prepare_memory(self, args, bytes)
        }
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
            diagnostic!("shift: {}: numeric argument required", args[0]);
            return 2;
        };
        if count < 0 || count as u64 > self.positional.len() as u64 {
            if count < 0 || self.shopt.shift_verbose {
                diagnostic!("shift: {count}: shift count out of range");
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
            *var.scalar_mut() = Some(value);
            var.declared = true;
            return Ok(());
        }
        Err("circular name reference")
    }

    pub fn reference_name<'a>(&self, name: &'a str) -> Result<Cow<'a, str>, &'static str> {
        self.scoped_reference_name(name, false)
    }

    pub(crate) fn read_reference_name<'a>(&self, name: &'a str) -> Option<Cow<'a, str>> {
        match self.reference_name(name) {
            Ok(target) => Some(target),
            Err(error) => {
                diagnostic!("warning: {name}: {error}");
                None
            }
        }
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

    pub(crate) fn ifs(&self) -> Cow<'_, str> {
        self.scalar_setting("IFS")
            .unwrap_or(Cow::Borrowed(crate::word::DEFAULT_IFS))
    }

    // Ordinary shell-authored scalars can be inspected without copying their value.
    pub(crate) fn scalar_setting(&self, name: &str) -> Option<Cow<'_, str>> {
        match self.variables.get(name) {
            Some(var) if !var.nameref && !var.dynamic && var.elements.is_none() => {
                return var.value.as_deref().map(Cow::Borrowed);
            }
            None => {
                // In-process script reentry changes only PATH and directory/level
                // variables. These inherited settings stay constant.
                static INHERITED: [std::sync::OnceLock<Option<String>>; 6] =
                    [const { std::sync::OnceLock::new() }; 6];
                let slot = match name {
                    "HISTSIZE" => Some(0),
                    "HISTCONTROL" => Some(1),
                    "HISTIGNORE" => Some(2),
                    "histchars" => Some(3),
                    "HOME" => Some(4),
                    "PROMPT_DIRTRIM" => Some(5),
                    "IFS" => return Some(Cow::Borrowed(crate::word::DEFAULT_IFS)),
                    _ => None,
                };
                if let Some(i) = slot {
                    return INHERITED[i]
                        .get_or_init(|| std::env::var(name).ok())
                        .as_deref()
                        .map(Cow::Borrowed);
                }
            }
            Some(_) => {}
        }
        self.get(name).map(Cow::Owned)
    }

    pub(crate) fn arithmetic_get(&self, name: &str) -> Result<Option<String>, &'static str> {
        if self.is_nameref(name)
            && let Cow::Owned(target) = self.reference_name(name)?
        {
            self.reference_value(name, &target)
        } else {
            Ok(self.get(name))
        }
    }

    fn reference_value(&self, name: &str, target: &str) -> Result<Option<String>, &'static str> {
        let depth = self.reference_depth.get();
        if depth >= 32 {
            return Err("array subscript nested too deeply");
        }
        self.reference_depth.set(depth + 1);
        let value = if !name.contains('[')
            && let Some((base, expr)) = array::subscript(target)
        {
            if self.is_associative(base) {
                crate::arithmetic::data_key(expr, &mut crate::arithmetic::ReadOnly(self))
                    .map(|key| self.assoc_value(base, &key).map(str::to_owned))
            } else {
                self.resolve_index(base, expr)
                    .map(|index| self.element(base, index))
            }
        } else {
            Ok(self.get(target))
        };
        self.reference_depth.set(depth);
        value
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
                return Some(map.ordered_keys().join("\0"));
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
            return self.reference_value(name, &target).ok().flatten();
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
        if name == "PIPESTATUS" || self.live_dirstack(name) {
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
            _ => match self.listed_environment.get() {
                Some(env) => env
                    .get(std::ffi::OsStr::new(name))?
                    .to_str()
                    .map(str::to_owned),
                None => std::env::var(name).ok(),
            },
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

    const BUILTIN_READONLY: [&str; 6] = [
        "BASHOPTS",
        "BASH_VERSINFO",
        "EUID",
        "PPID",
        "SHELLOPTS",
        "UID",
    ];

    fn builtin_readonly(name: &str) -> bool {
        Self::BUILTIN_READONLY.contains(&name)
    }

    pub fn initialize_shell_level(&mut self, env: &crate::startup::Environment) {
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
                | "BASH_XTRACEFD"
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

    pub(crate) fn capture_local(&self, name: &str) -> bool {
        is_identifier(name)
            && name.as_bytes()[0].is_ascii_lowercase()
            && !name.bytes().any(|b| b.is_ascii_uppercase())
            && !matches!(name, "histchars" | "auto_resume")
            && Self::ordinary_assignment(name)
            && self.variables.get(name).is_none_or(|v| {
                !v.readonly
                    && !v.integer
                    && !v.dynamic
                    && !v.nameref
                    && v.elements.is_none()
                    && v.letter_case.is_none()
            })
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
        let Some(value) = var.scalar_mut() else {
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
            "BASH_XTRACEFD"
                if !value.is_empty() && crate::execute::trace_descriptor(value).is_none() =>
            {
                diagnostic!("BASH_XTRACEFD: {value}: invalid value for trace file descriptor");
            }
            "GLOBIGNORE" => self.shopt.glob.set_ignore(Some(value)),
            "PATH" => self.hashes.clear(),
            _ => {}
        }
    }

    pub(crate) fn assign_value(&mut self, name: &str, mut value: Cow<'_, str>) {
        match self.reference_name(name) {
            Ok(Cow::Owned(_)) => {
                if let Err(e) = self.assign_target(name, &value) {
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
        let live = dynamic::is_dynamic(name) && self.variables.get(name).is_none_or(|v| v.dynamic);
        if let Some(upper) = self.variables.get(name).and_then(|v| v.letter_case) {
            value = Cow::Owned(self.convert_case(value.into_owned(), Some(upper)));
        }
        self.apply_variable_effects(name, &value);
        if matches!(name, "HISTSIZE" | "HISTFILESIZE") {
            self.apply_history_limit(name, &value);
        }
        if name == "BASH_ARGV0"
            && !self
                .locals
                .iter()
                .any(|scope| scope.variables.contains_key(name))
        {
            self.arg0 = value.to_string();
        }
        if live {
            self.set_dynamic(name, &value);
        }
        if let Some(var) = self.variables.value_mut(name, self.allexport) {
            if let Some(elements) = var.elements.as_mut().map(Rc::make_mut) {
                elements.insert(0, value.into_owned());
                var.array_assigned = true;
            } else if let (Some(current), Cow::Borrowed(text)) = (var.scalar_mut(), &value) {
                (*text).clone_into(current);
            } else {
                *var.scalar_mut() = Some(value.into_owned());
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
                integer: dynamic::integer_attribute(name),
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
        let text = var.scalar_mut().as_mut()?;
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
        if var.nameref {
            if append {
                return false;
            }
            // Copy the short resolved spelling before borrowing the destination
            // mutably. Longer and indexed references use normal assignment.
            let mut bytes = [0; 128];
            let Some(target) = self
                .scalar_reference_target(name)
                .filter(|s| s.len() <= bytes.len())
            else {
                return false;
            };
            let len = target.len();
            bytes[..len].copy_from_slice(target.as_bytes());
            return self
                .assign_existing_literal(std::str::from_utf8(&bytes[..len]).unwrap(), value);
        }
        if var.dynamic || var.letter_case.is_some() || var.readonly || var.elements.is_some() {
            return false;
        }
        let integer = var.integer;
        let Some(current) = var.scalar_mut().as_mut() else {
            return false;
        };
        if integer {
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
        self.remember_argument(words.last().map_or("", String::as_str));
    }

    pub(crate) fn remember_argument(&mut self, arg: &str) {
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

    // A scalar reference chain can be followed through borrowed table entries.
    // Indexed targets and virtual variables still use the general resolver.
    fn scalar_reference_target(&self, name: &str) -> Option<&str> {
        let mut target = self.variables.get(name)?.value.as_deref()?;
        for _ in 1..32 {
            if !is_identifier(target) {
                return None;
            }
            let Some(var) = self.variables.get(target).filter(|v| v.nameref) else {
                return Some(target);
            };
            target = var.value.as_deref()?;
        }
        None
    }

    pub(crate) fn referenced_scalar(&self, name: &str) -> Option<&str> {
        if self.is_nameref(name) {
            self.stored_scalar(self.scalar_reference_target(name)?)
        } else {
            self.stored_scalar(name)
        }
    }

    pub(crate) fn stored_char_count(&self, name: &str) -> Option<usize> {
        if !is_identifier(name)
            || matches!(
                name,
                "_" | "BASHPID" | "BASHOPTS" | "SHELLOPTS" | "PIPESTATUS"
            )
        {
            return None;
        }
        let var = self.variables.get(name)?;
        if var.nameref || var.dynamic || var.elements.is_some() {
            return None;
        }
        let cached = var.char_count.get();
        if cached != 0 {
            return Some((cached - 1) as usize);
        }
        let length = crate::word::character_count(var.value.as_deref()?);
        if let Ok(n) = u32::try_from(length + 1) {
            var.char_count.set(n);
        }
        Some(length)
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
            if self
                .variables
                .get(&name)
                .is_some_and(|v| v.origin == BindingOrigin::CommandPrefix)
            {
                for scope in self.locals.iter_mut().rev() {
                    if let Some(previous) = scope.variables.remove(&name)
                        && !previous
                            .as_ref()
                            .is_some_and(|v| v.origin == BindingOrigin::CommandPrefix)
                    {
                        break;
                    }
                }
            }
            let case = self.variables.get(&name).and_then(|v| v.letter_case);
            let value = self.convert_case(value, case);
            self.apply_variable_effects(&name, &value);
            let var = self.variables.entry(name).or_default();
            var.exported = true;
            var.declared = true;
            var.origin = BindingOrigin::Ordinary;
            if let Some(array) = var.elements.as_mut() {
                Rc::make_mut(array).insert(0, value);
            } else {
                *var.scalar_mut() = Some(value);
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
            let original = self.variables.insert(
                name.clone(),
                Variable {
                    origin: BindingOrigin::CommandPrefix,
                    declared: true,
                    value: Some(value.clone()),
                    exported: true,
                    ..Variable::default()
                },
            );
            saved.variables.entry(name.clone()).or_insert(original);
        }
        saved
    }

    pub fn restore(&mut self, bindings: Bindings) {
        if let Some(offset) = bindings.opt_offset {
            self.opt_offset = offset;
        }
        if bindings
            .variables
            .keys()
            .all(|name| Self::ordinary_assignment(name))
        {
            for (name, original) in bindings.variables {
                self.restore_binding(name, original);
            }
            return;
        }
        // Special bindings can change shell options. Keep their original order,
        // including when both IGNOREEOF and its lowercase alias leave scope.
        let mut sorted: Vec<_> = bindings.variables.into_iter().collect();
        sorted.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        for (name, original) in sorted {
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
            if matches!(name.as_str(), "HISTSIZE" | "HISTFILESIZE") {
                let value = original
                    .as_ref()
                    .and_then(|v| v.value.as_deref())
                    .map(Cow::Borrowed)
                    .or_else(|| {
                        original
                            .is_none()
                            .then(|| std::env::var(&name).ok().map(Cow::Owned))
                            .flatten()
                    });
                if let Some(value) = value {
                    self.apply_history_limit(&name, &value);
                }
            }
            if name == "PATH" {
                self.hashes.clear();
            }
            self.restore_binding(name, original);
        }
    }

    fn restore_binding(&mut self, name: String, original: Option<Variable>) {
        if let Some(var) = original {
            self.variables.insert(name, var);
        } else {
            self.variables.remove(&name);
        }
    }

    // Preserve arbitrary inherited environment bytes. Only shell-authored
    // changes and command-prefix assignments use the UTF-8 language model.
    pub fn environment_changes<'a>(
        &'a self,
        assignments: &'a [(String, String)],
    ) -> impl Iterator<Item = (&'a str, Option<Cow<'a, str>>)> {
        // Exported bindings from outer function scopes are still visible to a
        // child when an inner local has no exported value. Apply the scopes in
        // order so the nearest exported value (or command prefix) wins.
        let change = |name: &'a str, var: &'a Variable, outer: &[Bindings]| {
            if var.exported && var.elements.is_none() && var.value.is_some() {
                let value = match name {
                    "BASHOPTS" | "SHELLOPTS" => {
                        Some(Cow::Owned(self.option_list(name == "SHELLOPTS")))
                    }
                    _ => var.value.as_deref().map(Cow::Borrowed),
                };
                Some((name, value))
            } else if var.inherited(name) && !outer.iter().any(|s| s.variables.contains_key(name)) {
                Some((name, None))
            } else {
                None
            }
        };
        self.locals
            .iter()
            .enumerate()
            .flat_map(move |(i, scope)| {
                scope
                    .variables
                    .iter()
                    .filter_map(move |(name, var)| change(name, var.as_ref()?, &self.locals[..i]))
            })
            .chain(
                self.variables
                    .environment_entries()
                    .filter_map(move |(name, var)| change(name, var, &self.locals)),
            )
            .chain(self.function_environment.values().map(|export| {
                (
                    export.key.as_str(),
                    export.body.as_deref().map(Cow::Borrowed),
                )
            }))
            .chain(
                assignments
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(Cow::Borrowed(v.as_str())))),
            )
    }

    pub fn search_path<'a>(&'a self, assignments: &'a [(String, String)]) -> &'a std::ffi::OsStr {
        if let Some((_, path)) = assignments.iter().rev().find(|(k, _)| k == "PATH") {
            return path.as_ref();
        }
        if let Some(var) = self.variables.get("PATH") {
            return var.value.as_deref().unwrap_or("").as_ref();
        }
        self.inherited_path
            .get_or_init(|| {
                std::env::var_os("PATH")
                    .unwrap_or_else(|| self.get("PATH").unwrap_or_default().into())
            })
            .as_os_str()
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
