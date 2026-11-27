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

pub struct Invocation {
    pub options: StartupOptions,
    pub input: Input,
    pub arg0: String,
    pub positional: Vec<String>,
    pub interactive: bool,
    pub login: bool,
    pub as_sh: bool,
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
                        'n' => options.check = enabled,
                        'b' => options.notify = enabled,
                        'e' => options.errexit = enabled,
                        'v' => options.verbose = enabled,
                        'E' => options.errtrace = enabled,
                        'T' => options.functrace = enabled,
                        'C' => options.noclobber = enabled,
                        'P' => options.physical = enabled,
                        'k' => options.keyword = enabled,
                        'a' => options.allexport = enabled,
                        't' => options.onecmd = enabled,
                        'x' => options.xtrace = enabled,
                        'H' => options.histexpand = Some(enabled),
                        'm' => options.monitor = Some(enabled),
                        'h' => options.hashall = enabled,
                        'B' => options.braceexpand = enabled,
                        'f' => options.noglob = enabled,
                        'u' => options.nounset = enabled,
                        'O' => options
                            .shopts
                            .push((enabled, args.next().map(utf8).transpose()?)),
                        'o' => match args.next().as_deref().and_then(|a| a.to_str()) {
                            Some("braceexpand") => options.braceexpand = enabled,
                            Some("posix") => options.posix = Some(enabled),
                            Some("physical") => options.physical = enabled,
                            Some("nolog") => options.nolog = enabled,
                            Some("keyword") => options.keyword = enabled,
                            Some("interactive-comments") => options
                                .shopts
                                .push((enabled, Some("interactive_comments".into()))),
                            Some("pipefail") => options.pipefail = enabled,
                            Some("errexit") => options.errexit = enabled,
                            Some("verbose") => options.verbose = enabled,
                            Some("errtrace") => options.errtrace = enabled,
                            Some("functrace") => options.functrace = enabled,
                            Some("nounset") => options.nounset = enabled,
                            Some("allexport") => options.allexport = enabled,
                            Some("onecmd") => options.onecmd = enabled,
                            Some("xtrace") => options.xtrace = enabled,
                            Some("histexpand") => options.histexpand = Some(enabled),
                            Some("monitor") => options.monitor = Some(enabled),
                            Some("hashall") => options.hashall = enabled,
                            Some("noclobber") => options.noclobber = enabled,
                            Some("noexec") => options.check = enabled,
                            Some("notify") => options.notify = enabled,
                            Some("ignoreeof") => options.ignoreeof = Some(enabled),
                            Some("noglob") => options.noglob = enabled,
                            Some(name) => return Err(format!("{name}: invalid option name")),
                            None => return Err("expected an option name after -o".into()),
                        },
                        _ => return Err(format!("unsupported option: {arg}")),
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
        norc,
        noprofile,
        noediting,
        rcfile,
        pretty,
        debugger,
    }))
}
