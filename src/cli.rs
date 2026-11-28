use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub enum Input {
    Command(String),
    File(PathBuf),
    Stdin,
}

#[derive(Default)]
pub struct StartupOptions {
    pub check: bool,
    pub privileged: bool,
    pub history: Option<bool>,
    pub vi: Option<bool>,
    pub emacs: Option<bool>,
    pub notify: bool,
    pub ignoreeof: Option<bool>,
    pub errexit: bool,
    pub verbose: bool,
    pub errtrace: bool,
    pub functrace: bool,
    pub nounset: bool,
    pub noglob: bool,
    pub noclobber: bool,
    pub hashall: bool,
    pub allexport: bool,
    pub onecmd: bool,
    pub xtrace: bool,
    pub histexpand: Option<bool>,
    pub monitor: Option<bool>,
    pub braceexpand: bool,
    pub pipefail: bool,
    pub physical: bool,
    pub keyword: bool,
    pub nolog: bool,
    pub shopts: Vec<(bool, Option<String>)>,
    pub posix: Option<bool>,
}

impl StartupOptions {
    fn set(&mut self, name: &str, enabled: bool) -> Result<(), String> {
        match name {
                            "privileged" => self.privileged = enabled,
                            "history" => self.history = Some(enabled),
                            "vi" => {
                                self.vi = Some(enabled);
                                if enabled { self.emacs = Some(false); }
                            }
                            "emacs" => {
                                self.emacs = Some(enabled);
                                if enabled { self.vi = Some(false); }
                            }
                            "braceexpand" => self.braceexpand = enabled,
                            "posix" => self.posix = Some(enabled),
                            "physical" => self.physical = enabled,
                            "nolog" => self.nolog = enabled,
                            "keyword" => self.keyword = enabled,
                            "interactive-comments" => self
                                .shopts
                                .push((enabled, Some("interactive_comments".into()))),
                            "pipefail" => self.pipefail = enabled,
                            "errexit" => self.errexit = enabled,
                            "verbose" => self.verbose = enabled,
                            "errtrace" => self.errtrace = enabled,
                            "functrace" => self.functrace = enabled,
                            "nounset" => self.nounset = enabled,
                            "allexport" => self.allexport = enabled,
                            "onecmd" => self.onecmd = enabled,
                            "xtrace" => self.xtrace = enabled,
                            "histexpand" => self.histexpand = Some(enabled),
                            "monitor" => self.monitor = Some(enabled),
                            "hashall" => self.hashall = enabled,
                            "noclobber" => self.noclobber = enabled,
                            "noexec" => self.check = enabled,
                            "notify" => self.notify = enabled,
                            "ignoreeof" => self.ignoreeof = Some(enabled),
                            "noglob" => self.noglob = enabled,
            _ => return Err(format!("{name}: invalid option name")),
        }
        Ok(())
    }
}

pub struct Invocation {
    pub options: StartupOptions,
    pub input: Input,
    pub arg0: String,
    pub positional: Vec<String>,
    pub interactive: bool,
    pub login: bool,
    pub as_sh: bool,
    pub as_su: bool,
    pub norc: bool,
    pub noprofile: bool,
    pub noediting: bool,
    pub rcfile: Option<PathBuf>,
    pub pretty: bool,
    pub debugger: bool,
}

pub enum Action {
    Run(Invocation),
    Help,
    Version,
}

fn utf8(value: OsString) -> Result<String, String> {
    value
        .into_string()
        .map_err(|_| "expected a UTF-8 argument".into())
}

pub fn parse(
    mut args: impl Iterator<Item = OsString>,
    terminal: impl FnOnce() -> bool,
) -> Result<Action, String> {
    let name = args.next().unwrap_or_else(|| "tonari".into());
    let mut login = name.as_bytes().starts_with(b"-");
    let as_su = matches!(name.as_bytes().rsplit(|b| *b == b'/').next(), Some(b"su" | b"-su"));
    let as_sh = matches!(
        name.as_bytes().rsplit(|b| *b == b'/').next(),
        Some(b"sh" | b"-sh")
    );
    let mut options = StartupOptions {
        hashall: true,
        braceexpand: true,
        ..Default::default()
    };
    let mut args = args.peekable();
    let mut interactive = false;
    let mut command = false;
    let mut stdin = false;
    let mut norc = false;
    let mut noprofile = false;
    let mut noediting = false;
    let mut rcfile = None;
    let mut pretty = false;
    let mut debugger = false;
    while args
        .peek()
        .and_then(|a| a.to_str())
        .is_some_and(|s| s.starts_with('-') || (s.starts_with('+') && s.len() > 1))
    {
        let option = args.next().unwrap();
        let arg = option.to_str().unwrap();
        match arg {
            "--" | "-" => break,
            "--help" => return Ok(Action::Help),
            "--version" => return Ok(Action::Version),
            "--pretty-print" => pretty = true,
            "--debugger" => debugger = true,
            "--verbose" => options.verbose = true,
            "--posix" => options.posix = Some(true),
            "--login" => login = true,
            "--norc" => norc = true,
            "--noediting" => noediting = true,
            "--noprofile" => noprofile = true,
            "--rcfile" | "--init-file" => {
                rcfile = Some(PathBuf::from(
                    args.next().ok_or("--rcfile requires a file")?,
                ));
            }
            _ => {
                let enabled = arg.starts_with('-');
                for flag in arg[1..].chars() {
                    match flag {
                        'c' => command = true,
                        'i' => interactive = enabled,
                        'l' => login = enabled,
                        's' => stdin = enabled,
                        'O' => options
                            .shopts
                            .push((enabled, args.next().map(utf8).transpose()?)),
                        'o' => {
                            let name = utf8(args.next().ok_or("expected an option name after -o")?)?;
                            options.set(&name, enabled)?;
                        }
                        c => {
                            let name = crate::Session::short_option_name(c).ok_or_else(|| format!("unsupported option: {arg}"))?;
                            options.set(name, enabled)?;
                        }
                    }
                }
            }
        }
    }
    let mut arg0 = utf8(name)?;
    let input = if command {
        let source = utf8(args.next().ok_or("-c requires a UTF-8 command string")?)?;
        if let Some(name) = args.next() {
            arg0 = utf8(name)?;
        }
        Input::Command(source)
    } else if !stdin && args.peek().is_some() {
        let file = args.next().unwrap();
        arg0 = file.to_string_lossy().into_owned();
        Input::File(file.into())
    } else {
        interactive = interactive || terminal();
        Input::Stdin
    };
    let positional = args.map(utf8).collect::<Result<_, _>>()?;
    Ok(Action::Run(Invocation {
        options,
        input,
        arg0,
        positional,
        interactive,
        login,
        as_sh,
        as_su,
        norc,
        noprofile,
        noediting,
        rcfile,
        pretty,
        debugger,
    }))
}
