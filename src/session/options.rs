use std::io::{self, Write};

use super::Session;

#[derive(Clone)]
pub struct ShellOptions {
    pub compat: u8,
    pub autocd: bool,
    pub array_expand_once: bool,
    pub extquote: bool,
    pub cdable_vars: bool,
    pub checkhash: bool,
    pub nocasematch: bool,
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
            cdable_vars: false,
            checkhash: false,
            nocasematch: false,
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
}

macro_rules! option_entry {
    ($name:literal, $($field:ident).+) => {
        OptionEntry { name: $name, read: |s| s.$($field).+, write: Some(|s| &mut s.$($field).+) }
    };
}

const CURRENT_COMPAT: u8 = 53;
macro_rules! compat_entry {
    ($name:literal, $level:literal) => {
        OptionEntry {
            name: $name,
            read: |s| s.shopt.compat == $level,
            write: None,
        }
    };
}

// Derive both the lookup table and the direct snapshot reads from the same
// order. Function return only visits hooks for bits which actually changed.
macro_rules! set_options {
    ($( $name:literal => $($field:ident).+ ),+ $(,)?) => {
        const SET_OPTIONS: &[OptionEntry] = &[$(option_entry!($name, $($field).+)),+];
        fn set_bits(s: &Session) -> u32 {
            let mut bits = 0;
            for (i, enabled) in [$(s.$($field).+),+].into_iter().enumerate() {
                bits |= u32::from(enabled) << i;
            }
            bits
        }
    }
}
set_options! {
    "allexport" => allexport,
    "braceexpand" => shopt.braceexpand,
    "emacs" => emacs,
    "errexit" => errexit,
    "errtrace" => errtrace,
    "functrace" => functrace,
    "hashall" => hashes.enabled,
    "histexpand" => histexpand,
    "history" => history_enabled,
    "ignoreeof" => ignoreeof,
    "interactive-comments" => shopt.interactive_comments,
    "keyword" => keyword,
    "monitor" => jobs.monitor,
    "noclobber" => noclobber,
    "noexec" => noexec,
    "noglob" => noglob,
    "nolog" => shopt.nolog,
    "notify" => notify,
    "nounset" => nounset,
    "onecmd" => onecmd,
    "physical" => physical,
    "pipefail" => pipefail,
    "posix" => posix,
    "verbose" => verbose,
    "vi" => vi,
    "xtrace" => xtrace,
}
const SHOPT_OPTIONS: &[OptionEntry] = &[
    option_entry!("array_expand_once", shopt.array_expand_once),
    option_entry!("assoc_expand_once", shopt.array_expand_once),
    option_entry!("autocd", shopt.autocd),
    option_entry!("bash_source_fullpath", shopt.bash_source_fullpath),
    option_entry!("cdable_vars", shopt.cdable_vars),
    option_entry!("checkhash", shopt.checkhash),
    option_entry!("checkwinsize", shopt.checkwinsize),
    option_entry!("cmdhist", shopt.cmdhist),
    compat_entry!("compat31", 31),
    compat_entry!("compat32", 32),
    compat_entry!("compat40", 40),
    compat_entry!("compat41", 41),
    compat_entry!("compat42", 42),
    compat_entry!("compat43", 43),
    compat_entry!("compat44", 44),
    option_entry!("dotglob", shopt.glob.dotglob),
    option_entry!("execfail", shopt.execfail),
    option_entry!("expand_aliases", shopt.expand_aliases),
    option_entry!("extdebug", shopt.extdebug),
    option_entry!("extglob", shopt.glob.extglob),
    option_entry!("extquote", shopt.extquote),
    option_entry!("failglob", shopt.glob.failglob),
    option_entry!("force_fignore", shopt.force_fignore),
    option_entry!("globskipdots", shopt.glob.globskipdots),
    option_entry!("globstar", shopt.glob.globstar),
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
        read: |s| s.login,
        write: None,
    },
    option_entry!("nocaseglob", shopt.glob.nocaseglob),
    option_entry!("nocasematch", shopt.nocasematch),
    option_entry!("no_empty_cmd_completion", shopt.no_empty_cmd_completion),
    option_entry!("nullglob", shopt.glob.nullglob),
    option_entry!("patsub_replacement", shopt.patsub_replacement),
    option_entry!("progcomp", shopt.progcomp),
    option_entry!("progcomp_alias", shopt.progcomp_alias),
    option_entry!("promptvars", shopt.promptvars),
    option_entry!("shift_verbose", shopt.shift_verbose),
    option_entry!("sourcepath", shopt.sourcepath),
    option_entry!("varredir_close", shopt.varredir_close),
    option_entry!("xpg_echo", shopt.xpg_echo),
];
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
                eprintln!("tonari: BASH_COMPAT: {value}: compatibility value out of range");
                CURRENT_COMPAT
            }
        };
    }

    // Only the seven registered shopt names are accepted; numeric levels also
    // exist for releases without a corresponding option.
    fn toggle_compat(&mut self, name: &str, enable: bool) {
        let digits = &name["compat".len()..];
        if enable {
            self.assign("BASH_COMPAT", digits);
        } else {
            let active = self.shopt.compat;
            let level = digits.parse::<u8>().unwrap();
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

    pub(super) fn apply_posix_mode(&mut self, enabled: bool) {
        self.posix = enabled;
        self.shopt.shift_verbose = enabled;
        self.jobs.quiet = enabled && !self.interactive;
        self.shopt.expand_aliases = enabled || self.interactive;
        if enabled {
            self.shopt.inherit_errexit = true;
            self.shopt.sourcepath = true;
        }
    }

    pub(super) fn option_changed(&mut self, name: &str, enabled: bool) {
        use crate::editor::bindings::Keymap;
        if name == "noexec" && self.interactive {
            self.noexec = false;
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
        let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
        for fd in [libc::STDERR_FILENO, libc::STDIN_FILENO] {
            if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, size.as_mut_ptr()) } == -1 {
                continue;
            }
            let size = unsafe { size.assume_init() };
            let (rows, columns) = (size.ws_row, size.ws_col);
            if rows > 0 && columns > 0 && self.shopt.last_size != Some((rows, columns)) {
                self.assign("LINES", &rows.to_string());
                self.assign("COLUMNS", &columns.to_string());
                self.shopt.last_size = Some((rows, columns));
            }
            return Some((rows, columns));
        }
        None
    }

    fn option_entries(set: bool) -> &'static [OptionEntry] {
        if set { SET_OPTIONS } else { SHOPT_OPTIONS }
    }

    pub(super) fn save_local_options(&mut self) {
        if let Some(index) = self.declaration_scope("-")
            && self.locals[index].3.is_none()
        {
            self.locals[index].3 = std::num::NonZeroU32::new(set_bits(self) | (1 << 31));
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

    pub fn inherit_options(&mut self) {
        if let Ok(value) = std::env::var("BASH_COMPAT") {
            self.apply_compat(&value);
        }
        let allowed = std::cell::LazyCell::new(|| unsafe {
            libc::getuid() == libc::geteuid() && libc::getgid() == libc::getegid()
        });
        for (variable, set) in [("SHELLOPTS", true), ("BASHOPTS", false)] {
            if let Ok(list) = std::env::var(variable) {
                if !*allowed {
                    return;
                }
                for name in list.split(':') {
                    if !set
                        && name.starts_with("compat")
                        && SHOPT_OPTIONS.iter().any(|entry| entry.name == name)
                    {
                        self.toggle_compat(name, true);
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
        while let Some((option, rest)) = names.split_first() {
            if option == "--" {
                names = rest;
                break;
            }
            if !option.starts_with('-') || option == "-" {
                break;
            }
            for c in option[1..].chars() {
                match c {
                    's' => enable = true,
                    'u' => disable = true,
                    'p' => reusable = true,
                    'q' => quiet = true,
                    'o' => set = true,
                    _ => {
                        eprintln!("tonari: shopt: {option}: invalid option");
                        return 2;
                    }
                }
            }
            names = rest;
        }
        if enable && disable {
            eprintln!("tonari: shopt: cannot set and unset options simultaneously");
            return 1;
        }
        let mut status = 0;
        let mut out = io::stdout().lock();
        let all = names.is_empty();
        let known = if set { SET_OPTIONS } else { SHOPT_OPTIONS };
        let names = known
            .iter()
            .map(|entry| entry.name)
            .filter(|_| all)
            .chain(names.iter().map(String::as_str));
        for name in names {
            let Some(entry) = known.iter().find(|entry| entry.name == name) else {
                eprintln!("tonari: shopt: {name}: unsupported option");
                status = 1;
                continue;
            };
            if !all && (enable || disable) {
                if !set && name.starts_with("compat") {
                    self.toggle_compat(name, enable);
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
            let result = if reusable && set {
                writeln!(out, "set {}o {name}", if value { '-' } else { '+' })
            } else if reusable {
                writeln!(out, "shopt -{} {name}", if value { 's' } else { 'u' })
            } else {
                writeln!(out, "{name:15}\t{}", if value { "on" } else { "off" })
            };
            if result.is_err() {
                return 1;
            }
        }
        out.flush().map_or(1, |()| status)
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
