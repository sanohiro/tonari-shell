use super::{print_variable_value, set_value};
use std::io::{self, Write};

use super::Session;

#[derive(Clone)]
pub struct ShellOptions {
    pub compat: u8,
    pub autocd: bool,
    pub array_expand_once: bool,
    pub extquote: bool,
    pub gnu_errfmt: bool,
    pub globasciiranges: bool,
    pub cdable_vars: bool,
    pub cdspell: bool,
    pub dirspell: bool,
    pub direxpand: bool,
    pub complete_fullquote: bool,
    pub checkhash: bool,
    pub checkjobs: bool,
    pub mailwarn: bool,
    pub nocasematch: bool,
    pub noexpand_translation: bool,
    pub extdebug: bool,
    pub promptvars: bool,
    pub braceexpand: bool,
    pub lastpipe: bool,
    pub localvar_inherit: bool,
    pub localvar_unset: bool,
    pub inherit_errexit: bool,
    pub huponexit: bool,
    pub histappend: bool,
    pub cmdhist: bool,
    pub lithist: bool,
    pub histverify: bool,
    pub histreedit: bool,
    pub no_empty_cmd_completion: bool,
    pub nolog: bool,
    pub hostcomplete: bool,
    pub checkwinsize: bool,
    pub sourcepath: bool,
    pub bash_source_fullpath: bool,
    pub force_fignore: bool,
    pub interactive_comments: bool,
    pub shift_verbose: bool,
    pub execfail: bool,
    pub varredir_close: bool,
    pub xpg_echo: bool,
    pub expand_aliases: bool,
    pub progcomp: bool,
    pub progcomp_alias: bool,
    pub glob: crate::pattern::GlobOptions,
    pub patsub_replacement: bool,
    last_size: Option<(u16, u16)>,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            compat: CURRENT_COMPAT,
            autocd: false,
            array_expand_once: false,
            extquote: true,
            gnu_errfmt: false,
            globasciiranges: true,
            cdable_vars: false,
            cdspell: false,
            dirspell: false,
            direxpand: false,
            complete_fullquote: true,
            checkhash: false,
            checkjobs: false,
            mailwarn: false,
            nocasematch: false,
            noexpand_translation: false,
            extdebug: false,
            promptvars: true,
            braceexpand: true,
            lastpipe: false,
            localvar_inherit: false,
            localvar_unset: false,
            inherit_errexit: false,
            huponexit: false,
            histappend: false,
            cmdhist: true,
            lithist: false,
            histverify: false,
            histreedit: false,
            no_empty_cmd_completion: false,
            nolog: false,
            hostcomplete: true,
            checkwinsize: true,
            sourcepath: true,
            bash_source_fullpath: false,
            force_fignore: true,
            interactive_comments: true,
            shift_verbose: false,
            execfail: false,
            varredir_close: false,
            xpg_echo: false,
            expand_aliases: false,
            progcomp: true,
            progcomp_alias: false,
            glob: Default::default(),
            patsub_replacement: true,
            last_size: None,
        }
    }
}

pub(super) struct OptionEntry {
    pub name: &'static str,
    read: fn(&Session) -> bool,
    write: Option<fn(&mut Session) -> &mut bool>,
    compat: Option<u8>,
}

macro_rules! option_entry {
    ($name:literal, $($field:ident).+) => {
        OptionEntry { name: $name, read: |s| s.$($field).+, write: Some(|s| &mut s.$($field).+), compat: None }
    };
}

const CURRENT_COMPAT: u8 = 53;
macro_rules! compat_entry {
    ($name:literal, $level:literal) => {
        OptionEntry {
            name: $name,
            read: |s| s.shopt.compat == $level,
            compat: Some($level),
            write: None,
        }
    };
}

// Derive both the lookup table and the direct snapshot reads from the same
// order. Function return only visits hooks for bits which actually changed.
macro_rules! set_options {
    ($( $name:literal $( ( $short:literal ) )? => $($field:ident).+ ),+ $(,)?) => {
        const SET_OPTIONS: &[OptionEntry] = &[$(option_entry!($name, $($field).+)),+];
        const _: () = assert!(SET_OPTIONS.len() <= 31);
        impl Session {
            pub(crate) fn short_option_name(flag: char) -> Option<&'static str> {
                match flag {
                    $( $( $short => Some($name), )? )+
                    _ => None,
                }
            }
            fn short_enabled(&self, flag: char) -> bool {
                $( set_options!(@read self, flag, $($field).+ $(, $short)?); )+
                match flag {
                    'i' => self.interactive,
                    'c' => self.command_string,
                    's' => self.stdin_script,
                    _ => false,
                }
            }
        }
        fn set_short_option(s: &mut Session, flag: char, enabled: bool) -> bool {
            $( set_options!(@apply s, flag, enabled, $($field).+ $(, $short)?); )+
            false
        }
        fn set_bits(s: &Session) -> u32 {
            let mut bits = 0;
            for (i, enabled) in [$(s.$($field).+),+].into_iter().enumerate() {
                bits |= u32::from(enabled) << i;
            }
            bits
        }
    };
    (@read $s:ident, $flag:ident, $($field:ident).+, $short:literal) => {
        if $flag == $short { return $s.$($field).+; }
    };
    (@read $s:ident, $flag:ident, $($field:ident).+) => {};
    (@apply $s:ident, $flag:ident, $enabled:ident, $($field:ident).+, $short:literal) => {
        if $flag == $short { $s.$($field).+ = $enabled; return true; }
    };
    (@apply $s:ident, $flag:ident, $enabled:ident, $($field:ident).+) => {};
}
set_options! {
    "allexport" ('a') => allexport,
    "braceexpand" ('B') => shopt.braceexpand,
    "emacs" => emacs,
    "errexit" ('e') => errexit,
    "errtrace" ('E') => errtrace,
    "functrace" ('T') => functrace,
    "hashall" ('h') => hashes.enabled,
    "histexpand" ('H') => histexpand,
    "history" => history_enabled,
    "ignoreeof" => ignoreeof,
    "interactive-comments" => shopt.interactive_comments,
    "keyword" ('k') => keyword,
    "monitor" ('m') => jobs.monitor,
    "noclobber" ('C') => noclobber,
    "noexec" ('n') => noexec,
    "noglob" ('f') => noglob,
    "nolog" => shopt.nolog,
    "notify" ('b') => jobs.notify,
    "nounset" ('u') => nounset,
    "onecmd" ('t') => onecmd,
    "physical" ('P') => physical,
    "pipefail" => pipefail,
    "posix" => posix,
    "privileged" ('p') => privileged,
    "verbose" ('v') => verbose,
    "vi" => vi,
    "xtrace" ('x') => xtrace,
}
const SHOPT_OPTIONS: &[OptionEntry] = &[
    option_entry!("array_expand_once", shopt.array_expand_once),
    option_entry!("assoc_expand_once", shopt.array_expand_once),
    option_entry!("autocd", shopt.autocd),
    option_entry!("bash_source_fullpath", shopt.bash_source_fullpath),
    option_entry!("cdable_vars", shopt.cdable_vars),
    option_entry!("cdspell", shopt.cdspell),
    option_entry!("checkhash", shopt.checkhash),
    option_entry!("checkjobs", shopt.checkjobs),
    option_entry!("checkwinsize", shopt.checkwinsize),
    option_entry!("cmdhist", shopt.cmdhist),
    compat_entry!("compat31", 31),
    compat_entry!("compat32", 32),
    compat_entry!("compat40", 40),
    compat_entry!("compat41", 41),
    compat_entry!("compat42", 42),
    compat_entry!("compat43", 43),
    compat_entry!("compat44", 44),
    option_entry!("complete_fullquote", shopt.complete_fullquote),
    option_entry!("direxpand", shopt.direxpand),
    option_entry!("dirspell", shopt.dirspell),
    option_entry!("dotglob", shopt.glob.dotglob),
    option_entry!("execfail", shopt.execfail),
    option_entry!("expand_aliases", shopt.expand_aliases),
    option_entry!("extdebug", shopt.extdebug),
    option_entry!("extglob", shopt.glob.extglob),
    option_entry!("extquote", shopt.extquote),
    option_entry!("failglob", shopt.glob.failglob),
    option_entry!("force_fignore", shopt.force_fignore),
    option_entry!("globasciiranges", shopt.globasciiranges),
    option_entry!("globskipdots", shopt.glob.globskipdots),
    option_entry!("globstar", shopt.glob.globstar),
    option_entry!("gnu_errfmt", shopt.gnu_errfmt),
    option_entry!("histappend", shopt.histappend),
    option_entry!("histreedit", shopt.histreedit),
    option_entry!("histverify", shopt.histverify),
    option_entry!("hostcomplete", shopt.hostcomplete),
    option_entry!("huponexit", shopt.huponexit),
    option_entry!("inherit_errexit", shopt.inherit_errexit),
    option_entry!("interactive_comments", shopt.interactive_comments),
    option_entry!("lastpipe", shopt.lastpipe),
    option_entry!("lithist", shopt.lithist),
    option_entry!("localvar_inherit", shopt.localvar_inherit),
    option_entry!("localvar_unset", shopt.localvar_unset),
    OptionEntry {
        name: "login_shell",
        compat: None,
        read: |s| s.login,
        write: None,
    },
    option_entry!("mailwarn", shopt.mailwarn),
    option_entry!("no_empty_cmd_completion", shopt.no_empty_cmd_completion),
    option_entry!("nocaseglob", shopt.glob.nocaseglob),
    option_entry!("nocasematch", shopt.nocasematch),
    option_entry!("noexpand_translation", shopt.noexpand_translation),
    option_entry!("nullglob", shopt.glob.nullglob),
    option_entry!("patsub_replacement", shopt.patsub_replacement),
    option_entry!("progcomp", shopt.progcomp),
    option_entry!("progcomp_alias", shopt.progcomp_alias),
    option_entry!("promptvars", shopt.promptvars),
    OptionEntry {
        name: "restricted_shell",
        compat: None,
        read: |_| false,
        write: None,
    },
    option_entry!("shift_verbose", shopt.shift_verbose),
    option_entry!("sourcepath", shopt.sourcepath),
    option_entry!("varredir_close", shopt.varredir_close),
    option_entry!("xpg_echo", shopt.xpg_echo),
];
fn write_option_line(
    out: &mut dyn Write,
    name: &str,
    value: bool,
    reusable: bool,
    width: usize,
) -> io::Result<()> {
    if reusable {
        writeln!(out, "set {}o {name}", if value { '-' } else { '+' })
    } else {
        writeln!(out, "{name:width$}\t{}", if value { "on" } else { "off" })
    }
}

impl Session {
    pub(super) fn apply_compat(&mut self, value: &str) {
        self.shopt.compat = if value.is_empty() {
            CURRENT_COMPAT
        } else {
            let digits = match value.as_bytes() {
                [a, b] | [a, b'.', b] if a.is_ascii_digit() && b.is_ascii_digit() => {
                    (a - b'0') * 10 + b - b'0'
                }
                _ => 0,
            };
            if (31..=CURRENT_COMPAT).contains(&digits) {
                digits
            } else {
                diagnostic!("BASH_COMPAT: {value}: compatibility value out of range");
                CURRENT_COMPAT
            }
        };
    }

    // Only the seven registered shopt names are accepted; numeric levels also
    // exist for releases without a corresponding option.
    fn toggle_compat(&mut self, level: u8, enable: bool) {
        if enable {
            self.assign("BASH_COMPAT", itoa::Buffer::new().format(level));
        } else {
            let active = self.shopt.compat;
            self.assign(
                "BASH_COMPAT",
                &if level == active {
                    CURRENT_COMPAT
                } else {
                    active
                }
                .to_string(),
            );
        }
    }

    pub fn set_posix(&mut self, enabled: bool) {
        if enabled && self.get("POSIXLY_CORRECT").is_none() {
            self.assign("POSIXLY_CORRECT", "y");
        } else if !enabled && self.get("POSIXLY_CORRECT").is_some() {
            self.unset_variable("POSIXLY_CORRECT");
        }
        self.apply_posix_mode(enabled);
    }

    pub(crate) fn apply_posix_mode(&mut self, enabled: bool) {
        self.posix = enabled;
        self.shopt.shift_verbose = enabled;
        self.jobs.quiet = enabled && !self.interactive;
        self.jobs.posix_interactive = enabled && self.interactive;
        self.shopt.expand_aliases = enabled || self.interactive;
        if enabled {
            self.shopt.interactive_comments = true;
            self.shopt.inherit_errexit = true;
            self.shopt.sourcepath = true;
        }
    }

    pub(super) fn option_changed(&mut self, name: &str, enabled: bool) {
        use crate::editor::bindings::Keymap;
        if name == "noexec" && self.interactive {
            self.noexec = false;
        }
        if name == "monitor" && enabled {
            self.jobs.init(self.interactive);
        }
        if name == "history" && enabled && !self.interactive && self.history_lines().is_empty() {
            crate::history::load_script_history(self);
        }
        if name == "histexpand" {
            self.command_history_expansion = enabled;
        }
        if name == "posix" {
            self.set_posix(enabled);
        }
        if name == "ignoreeof" {
            self.unset_variable("ignoreeof");
            if enabled {
                self.assign("IGNOREEOF", "10");
            } else {
                self.unset_variable("IGNOREEOF");
            }
        }
        if enabled && name == "emacs" {
            self.vi = false;
            self.bindings.map = Keymap::Emacs;
        }
        if enabled && name == "vi" {
            self.emacs = false;
            self.bindings.map = Keymap::Insert;
        }
    }
    pub(crate) fn set_eof_variable(&mut self, value: Option<&str>) {
        self.ignoreeof = value.is_some();
        let Some(text) = value else { return };
        self.ignoreeof_limit = Some(text)
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
    }

    pub(crate) fn read_comments(&self) -> bool {
        !self.interactive || self.source_depth != 0 || self.shopt.interactive_comments
    }

    pub fn refresh_terminal_size(&mut self) -> Option<(u16, u16)> {
        if !self.interactive || !self.shopt.checkwinsize {
            return None;
        }
        let (rows, columns) = [libc::STDERR_FILENO, libc::STDIN_FILENO]
            .into_iter()
            .find_map(crate::interactive::terminal_size)?;
        if rows > 0 && columns > 0 && self.shopt.last_size != Some((rows, columns)) {
            self.assign("LINES", &rows.to_string());
            self.assign("COLUMNS", &columns.to_string());
            self.shopt.last_size = Some((rows, columns));
        }
        Some((rows, columns))
    }

    fn option_entries(set: bool) -> &'static [OptionEntry] {
        if set { SET_OPTIONS } else { SHOPT_OPTIONS }
    }

    pub(super) fn save_local_options(&mut self) {
        if let Some(index) = self.declaration_scope("-")
            && self.locals[index].set_options.is_none()
        {
            self.locals[index].set_options = std::num::NonZeroU32::new(set_bits(self) | (1 << 31));
        }
    }

    pub(super) fn restore_set_options(&mut self, bits: u32) {
        let mut changed = set_bits(self) ^ bits;
        while changed != 0 {
            let index = changed.trailing_zeros();
            changed &= changed - 1;
            let entry = &SET_OPTIONS[index as usize];
            let enabled = bits & (1 << index) != 0;
            if let Some(write) = entry.write {
                *write(self) = enabled;
                self.option_changed(entry.name, enabled);
            }
        }
    }

    pub(crate) fn options(&self, set: bool) -> impl Iterator<Item = (&'static str, bool)> {
        Self::option_entries(set)
            .iter()
            .map(move |entry| (entry.name, (entry.read)(self)))
    }

    pub(crate) fn set_option_enabled(&self, name: &str) -> bool {
        SET_OPTIONS
            .binary_search_by_key(&name, |entry| entry.name)
            .is_ok_and(|i| (SET_OPTIONS[i].read)(self))
    }

    pub(super) fn option_list(&self, set: bool) -> String {
        let mut result = String::new();
        for (name, enabled) in self.options(set) {
            if enabled {
                if !result.is_empty() {
                    result.push(':');
                }
                result.push_str(name);
            }
        }
        result
    }

    pub fn inherit_options(&mut self, env: &crate::startup::Environment) {
        if let Some(value) = &env.compat {
            self.apply_compat(value);
        }

        for (variable, set, list) in [
            ("SHELLOPTS", true, &env.shellopts),
            ("BASHOPTS", false, &env.bashopts),
        ] {
            if let Some(list) = list {
                for name in list.split(':') {
                    if !set
                        && let Some(level) = SHOPT_OPTIONS
                            .iter()
                            .find(|entry| entry.name == name)
                            .and_then(|entry| entry.compat)
                    {
                        self.toggle_compat(level, true);
                    } else if let Some(value) = self.option_value(name, set) {
                        *value = true;
                        if set {
                            self.option_changed(name, true);
                        }
                    }
                }
                let var = self.declared_variable(variable, false).unwrap();
                self.variables.insert(variable.into(), var);
            }
        }
    }

    pub(super) fn option_value(&mut self, name: &str, set: bool) -> Option<&mut bool> {
        Self::option_entries(set)
            .iter()
            .find(|entry| entry.name == name)
            .and_then(|entry| entry.write.map(|write| write(self)))
    }

    pub fn shopt(&mut self, args: &[String]) -> u8 {
        let (mut enable, mut disable, mut reusable, mut quiet, mut set) =
            (false, false, false, false, false);
        let mut names = args;
        while let Some(option) = crate::builtins::flags::next(&mut names) {
            for c in option[1..].chars() {
                match c {
                    's' => enable = true,
                    'u' => disable = true,
                    'p' => reusable = true,
                    'q' => quiet = true,
                    'o' => set = true,
                    _ => {
                        diagnostic!("shopt: {option}: invalid option");
                        return 2;
                    }
                }
            }
        }
        if enable && disable {
            diagnostic!("shopt: cannot set and unset options simultaneously");
            return 1;
        }
        let mut status = 0;
        let mut out: Option<std::io::BufWriter<_>> = None;
        let all = names.is_empty();
        let known = if set { SET_OPTIONS } else { SHOPT_OPTIONS };
        let requested = names.iter().map(|name| {
            known
                .binary_search_by_key(&name.as_str(), |entry| entry.name)
                .map(|i| &known[i])
                .map_err(|_| name)
        });
        let listed = if all { known } else { &[] };
        for entry in listed.iter().map(Ok).chain(requested) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(name) => {
                    if out.as_mut().is_some_and(|w| w.flush().is_err()) {
                        return 1;
                    }
                    diagnostic!("shopt: {name}: unsupported option");
                    status = 1;
                    continue;
                }
            };
            let name = entry.name;
            if !all && (enable || disable) {
                if !set && let Some(level) = entry.compat {
                    self.toggle_compat(level, enable);
                } else if let Some(write) = entry.write {
                    if name == "hostcomplete" && !set && !self.word_breaks_unset {
                        let breaks = self.completion_breaks();
                        if enable != breaks.contains('@') {
                            let adjusted = if enable {
                                format!("@{breaks}")
                            } else {
                                breaks.replace('@', "")
                            };
                            self.assign("COMP_WORDBREAKS", &adjusted);
                        }
                    }
                    *write(self) = enable;
                    if !set && name == "globasciiranges" {
                        crate::pattern::sync_ranges(self);
                    }
                    if set {
                        self.option_changed(name, enable);
                    }
                    if name == "extdebug" && !set {
                        if enable {
                            self.debug_base
                                .get_or_init(|| self.positional.as_slice().to_vec());
                        }
                        self.errtrace = enable;
                        self.functrace = enable;
                    }
                }
                continue;
            }
            let value = (entry.read)(self);
            if !all && !value {
                status = 1;
            }
            if quiet || (enable && !value) || (disable && value) {
                continue;
            }
            let writer = out.get_or_insert_with(|| crate::builtins::listing_stdout("shopt"));
            let result = if reusable && !set {
                writeln!(writer, "shopt -{} {name}", if value { 's' } else { 'u' })
            } else {
                write_option_line(
                    writer,
                    name,
                    value,
                    reusable,
                    if set && all { 15 } else { 20 },
                )
            };
            if result.is_err() {
                return 1;
            }
        }
        out.map_or(Ok(()), |mut out| out.flush())
            .map_or(1, |()| status)
    }
}

impl Session {
    pub fn option_flags(&self) -> String {
        "abefhikmnptuvxBCEHPTcs"
            .chars()
            .filter(|&flag| self.short_enabled(flag))
            .collect()
    }

    pub(super) fn empty_array_listing(&self, name: &str) -> bool {
        match name {
            "BASH_ARGV" | "BASH_ARGC" => {
                self.function_depth == 0 && self.debug_base.get().is_none()
            }
            "BASH_ALIASES" => self
                .aliases
                .0
                .as_ref()
                .is_none_or(|aliases| aliases.is_empty()),
            "BASH_CMDS" => self.hashes.items().next().is_none(),
            "BASH_SOURCE" => self.source_names.is_empty(),
            "BASH_LINENO" => self.call_lines.is_empty(),
            "GROUPS" => !self.dynamic.groups_read.get(),
            "DIRSTACK" => !self.dynamic.stack_read.get(),
            _ => false,
        }
    }

    pub fn set(&mut self, words: &mut Vec<String>) -> u8 {
        let args = words.get(1..).unwrap_or_default();
        if args.is_empty() {
            let mut out = io::BufWriter::new(crate::builtins::StdoutFd(Some("set")));
            let locale = self.character_locale();
            self.listed_environment();
            for name in self.visible_names("", super::NameList::Declared) {
                let stored = self.variables.get(name);
                let snapshot;
                let empty_snapshot = stored.is_none() && self.empty_array_listing(name);
                let result = if name == "BASH_VERSINFO" && stored.is_none() {
                    // The executable version is immutable across shell scopes.
                    static VERSION: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
                    out.write_all(VERSION.get_or_init(|| {
                        let mut bytes = name.as_bytes().to_vec();
                        let version = self.declared_variable(name, false).unwrap();
                        print_variable_value(&mut bytes, &version, locale)
                            .expect("in-memory write");
                        bytes
                    }))
                } else if empty_snapshot {
                    writeln!(out, "{name}=()")
                } else if self.array_parameter(name) {
                    let var = if let Some(var) = stored.filter(|v| !v.dynamic) {
                        var
                    } else {
                        snapshot = self.declared_variable(name, false).unwrap();
                        &snapshot
                    };
                    if !var.array_assigned
                        || name == "PIPESTATUS" && self.pipe_status.first.is_none()
                    {
                        continue;
                    }
                    write!(out, "{name}").and_then(|_| print_variable_value(&mut out, var, locale))
                } else {
                    let value = self.listed_scalar(name, stored);
                    let Some(value) = value else { continue };
                    writeln!(out, "{name}={}", set_value(&value, self.posix, locale))
                };
                if result.is_err() {
                    return 1;
                }
            }
            if !self.posix {
                for name in self.functions.names("") {
                    if self.print_function(&mut out, name, false).is_err() {
                        return 1;
                    }
                }
            }
            return out.flush().map_or(1, |()| 0);
        }
        let mut i = 0;
        // GNU set validates short flags before applying any. A bad name after
        // -o is diagnosed during application and preserves preceding changes.
        let has_option_name = |name: &String| !name.is_empty() && !name.starts_with(['-', '+']);
        let mut flags = args.iter().peekable();
        while let Some(opt) = flags.next() {
            let Some((prefix @ ("-" | "+"), rest)) = opt.split_at_checked(1) else {
                break;
            };
            if matches!(opt.as_str(), "--" | "-") {
                break;
            }
            for flag in rest.chars() {
                if flag == 'o' {
                    flags.next_if(|name| !name.starts_with(['-', '+']) || name.len() == 1);
                } else if Self::short_option_name(flag).is_none() {
                    diagnostic!("set: {prefix}{flag}: invalid option");
                    return 2;
                }
            }
        }
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
                    'n' => self.noexec = enable && !self.interactive,
                    'H' => {
                        self.histexpand = enable;
                        self.command_history_expansion = enable;
                    }
                    'o' => {
                        let Some(name) = args.get(i).filter(|name| has_option_name(name)) else {
                            let mut out = io::BufWriter::with_capacity(
                                512,
                                crate::builtins::StdoutFd(Some("set")),
                            );
                            for (name, on) in self.options(true) {
                                let result = write_option_line(&mut out, name, on, !enable, 15);
                                if result.is_err() {
                                    return 1;
                                }
                            }
                            if out.flush().is_err() {
                                return 1;
                            }
                            continue;
                        };
                        i += 1;
                        if let Some(value) = self.option_value(name, true) {
                            *value = enable;
                            self.option_changed(name, enable);
                        } else {
                            diagnostic!("set: {name}: unsupported option");
                            return 2;
                        }
                    }
                    'm' => {
                        self.jobs.monitor = enable;
                        self.option_changed("monitor", enable);
                    }
                    short if set_short_option(self, short, enable) => {}
                    _ => {
                        diagnostic!("set: {opt}: unsupported option");
                        return 2;
                    }
                }
            }
        }
        if i < args.len() || replace_positionals {
            words.drain(..i + 1);
            self.positional = std::mem::take(words).into_iter();
            self.source_set_args = true;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::Session;

    fn options(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn shopt_control_and_query_share_the_actual_shell_options() {
        let mut s = Session::default();
        assert_eq!(s.shopt(&options("-q sourcepath")), 0);
        assert_eq!(s.shopt(&options("-u sourcepath")), 0);
        assert!(!s.shopt.sourcepath);
        assert_eq!(s.shopt(&options("-q checkwinsize sourcepath")), 1);
        assert_eq!(s.shopt(&options("-so pipefail")), 0);
        assert!(s.pipefail);
    }
}
