use std::io::{self, Write};

use super::Session;

#[derive(Clone)]
pub struct ShellOptions {
    pub braceexpand: bool,
    pub localvar_inherit: bool,
    pub checkwinsize: bool,
    pub sourcepath: bool,
    pub expand_aliases: bool,
    pub progcomp: bool,
    pub glob: crate::pattern::GlobOptions,
    pub patsub_replacement: bool,
    last_size: Option<(u16, u16)>,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            braceexpand: true,
            localvar_inherit: false,
            checkwinsize: true,
            sourcepath: true,
            expand_aliases: false,
            progcomp: true,
            glob: Default::default(),
            patsub_replacement: true,
            last_size: None,
        }
    }
}

const SET_OPTIONS: &[&str] = &[
    "braceexpand",
    "errexit",
    "hashall",
    "monitor",
    "noclobber",
    "noglob",
    "nounset",
    "pipefail",
];
const SHOPT_OPTIONS: &[&str] = &[
    "checkwinsize",
    "dotglob",
    "expand_aliases",
    "extglob",
    "globskipdots",
    "globstar",
    "localvar_inherit",
    "nullglob",
    "patsub_replacement",
    "progcomp",
    "sourcepath",
];

impl Session {
    pub fn refresh_terminal_size(&mut self) {
        if !self.interactive || !self.shopt.checkwinsize {
            return;
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
            break;
        }
    }

    pub(super) fn option_value(&mut self, name: &str, set: bool) -> Option<&mut bool> {
        match (set, name) {
            (true, "braceexpand") => Some(&mut self.shopt.braceexpand),
            (true, "hashall") => Some(&mut self.hashes.enabled),
            (true, "errexit") => Some(&mut self.errexit),
            (true, "monitor") => Some(&mut self.jobs.monitor),
            (true, "noclobber") => Some(&mut self.noclobber),
            (true, "noglob") => Some(&mut self.noglob),
            (true, "nounset") => Some(&mut self.nounset),
            (true, "pipefail") => Some(&mut self.pipefail),
            (false, "dotglob") => Some(&mut self.shopt.glob.dotglob),
            (false, "extglob") => Some(&mut self.shopt.glob.extglob),
            (false, "globskipdots") => Some(&mut self.shopt.glob.globskipdots),
            (false, "globstar") => Some(&mut self.shopt.glob.globstar),
            (false, "localvar_inherit") => Some(&mut self.shopt.localvar_inherit),
            (false, "nullglob") => Some(&mut self.shopt.glob.nullglob),
            (false, "checkwinsize") => Some(&mut self.shopt.checkwinsize),
            (false, "patsub_replacement") => Some(&mut self.shopt.patsub_replacement),
            (false, "progcomp") => Some(&mut self.shopt.progcomp),
            (false, "expand_aliases") => Some(&mut self.shopt.expand_aliases),
            (false, "sourcepath") => Some(&mut self.shopt.sourcepath),
            _ => None,
        }
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
            .copied()
            .filter(|_| all)
            .chain(names.iter().map(String::as_str));
        for name in names {
            let Some(value) = self.option_value(name, set) else {
                eprintln!("tonari: shopt: {name}: unsupported option");
                status = 1;
                continue;
            };
            if !all && (enable || disable) {
                *value = enable;
                continue;
            }
            if !all && !*value {
                status = 1;
            }
            if quiet || (enable && !*value) || (disable && *value) {
                continue;
            }
            let result = if reusable && set {
                writeln!(out, "set {}o {name}", if *value { '-' } else { '+' })
            } else if reusable {
                writeln!(out, "shopt -{} {name}", if *value { 's' } else { 'u' })
            } else {
                writeln!(out, "{name:15}\t{}", if *value { "on" } else { "off" })
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
