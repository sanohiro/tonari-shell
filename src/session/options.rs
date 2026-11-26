use std::io::{self, Write};

use super::Session;

#[derive(Clone)]
pub struct ShellOptions {
    pub autocd: bool,
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
    pub hostcomplete: bool,
    pub checkwinsize: bool,
    pub sourcepath: bool,
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
            autocd: false,
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
            hostcomplete: true,
            checkwinsize: true,
            sourcepath: true,
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

const SET_OPTIONS: &[OptionEntry] = &[
    option_entry!("allexport", allexport),
    option_entry!("braceexpand", shopt.braceexpand),
    option_entry!("emacs", emacs),
    option_entry!("errexit", errexit),
    option_entry!("errtrace", errtrace),
    option_entry!("functrace", functrace),
    option_entry!("hashall", hashes.enabled),
    option_entry!("histexpand", histexpand),
    option_entry!("history", history_enabled),
    option_entry!("ignoreeof", ignoreeof),
    option_entry!("monitor", jobs.monitor),
    option_entry!("noclobber", noclobber),
    option_entry!("noexec", noexec),
    option_entry!("noglob", noglob),
    option_entry!("notify", notify),
    option_entry!("nounset", nounset),
    option_entry!("onecmd", onecmd),
    option_entry!("pipefail", pipefail),
    option_entry!("posix", posix),
    option_entry!("verbose", verbose),
    option_entry!("vi", vi),
    option_entry!("xtrace", xtrace),
];
const SHOPT_OPTIONS: &[OptionEntry] = &[
    option_entry!("autocd", shopt.autocd),
    option_entry!("cdable_vars", shopt.cdable_vars),
    option_entry!("checkhash", shopt.checkhash),
    option_entry!("checkwinsize", shopt.checkwinsize),
    option_entry!("dotglob", shopt.glob.dotglob),
    option_entry!("expand_aliases", shopt.expand_aliases),
    option_entry!("extdebug", shopt.extdebug),
    option_entry!("extglob", shopt.glob.extglob),
    option_entry!("failglob", shopt.glob.failglob),
    option_entry!("globskipdots", shopt.glob.globskipdots),
    option_entry!("globstar", shopt.glob.globstar),
    option_entry!("histappend", shopt.histappend),
    option_entry!("hostcomplete", shopt.hostcomplete),
    option_entry!("huponexit", shopt.huponexit),
    option_entry!("inherit_errexit", shopt.inherit_errexit),
    option_entry!("lastpipe", shopt.lastpipe),
    option_entry!("localvar_inherit", shopt.localvar_inherit),
    option_entry!("localvar_unset", shopt.localvar_unset),
    OptionEntry {
        name: "login_shell",
        read: |s| s.login,
        write: None,
    },
    option_entry!("nocaseglob", shopt.glob.nocaseglob),
    option_entry!("nocasematch", shopt.nocasematch),
    option_entry!("nullglob", shopt.glob.nullglob),
    option_entry!("patsub_replacement", shopt.patsub_replacement),
    option_entry!("progcomp", shopt.progcomp),
    option_entry!("progcomp_alias", shopt.progcomp_alias),
    option_entry!("promptvars", shopt.promptvars),
    option_entry!("sourcepath", shopt.sourcepath),
    option_entry!("varredir_close", shopt.varredir_close),
    option_entry!("xpg_echo", shopt.xpg_echo),
];
impl Session {
    pub fn set_posix(&mut self, enabled: bool) {
        self.posix = enabled;
        self.jobs.quiet = enabled && !self.interactive;
        self.shopt.expand_aliases = enabled || self.interactive;
        if enabled {
            self.shopt.inherit_errexit = true;
        }
    }

    pub(super) fn option_changed(&mut self, name: &str, enabled: bool) {
        use crate::editor::bindings::Keymap;
        if name == "noexec" && self.interactive {
            self.noexec = false;
        }
        if name == "histexpand" {
            self.command_history_expansion = enabled;
        }
        if name == "posix" {
            self.set_posix(enabled);
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
        let allowed = std::cell::LazyCell::new(|| unsafe {
            libc::getuid() == libc::geteuid() && libc::getgid() == libc::getegid()
        });
        for (variable, set) in [("SHELLOPTS", true), ("BASHOPTS", false)] {
            if let Ok(list) = std::env::var(variable) {
                if !*allowed {
                    return;
                }
                for name in list.split(':') {
                    if let Some(value) = self.option_value(name, set) {
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
                if let Some(write) = entry.write {
                    *write(self) = enable;
                    if set {
                        self.option_changed(name, enable);
                    }
                    if name == "extdebug" && !set {
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
