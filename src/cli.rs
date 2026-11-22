use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub enum Input {
    Command(String),
    File(PathBuf),
    Stdin,
}

pub struct Invocation {
    pub input: Input,
    pub arg0: String,
    pub positional: Vec<String>,
    pub interactive: bool,
    pub login: bool,
    pub norc: bool,
    pub noprofile: bool,
    pub noediting: bool,
    pub rcfile: Option<PathBuf>,
    pub check: bool,
    pub errexit: bool,
    pub verbose: bool,
    pub errtrace: bool,
    pub functrace: bool,
    pub nounset: bool,
    pub noglob: bool,
    pub noclobber: bool,
    pub monitor: Option<bool>,
    pub braceexpand: bool,
    pub pipefail: bool,
    pub shopts: Vec<(bool, Option<String>)>,
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
    let mut args = args.peekable();
    let mut interactive = false;
    let mut command = false;
    let mut stdin = false;
    let mut norc = false;
    let mut noprofile = false;
    let mut noediting = false;
    let mut rcfile = None;
    let mut check = false;
    let mut errexit = false;
    let mut verbose = false;
    let mut errtrace = false;
    let mut functrace = false;
    let mut nounset = false;
    let mut noglob = false;
    let mut noclobber = false;
    let mut monitor = None;
    let mut braceexpand = true;
    let mut pipefail = false;
    let mut shopts = Vec::new();
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
                        'c' if enabled => command = true,
                        'i' => interactive = enabled,
                        'l' => login = enabled,
                        's' => stdin = enabled,
                        'n' => check = enabled,
                        'e' => errexit = enabled,
                        'v' => verbose = enabled,
                        'E' => errtrace = enabled,
                        'T' => functrace = enabled,
                        'C' => noclobber = enabled,
                        'm' => monitor = Some(enabled),
                        'B' => braceexpand = enabled,
                        'f' => noglob = enabled,
                        'u' => nounset = enabled,
                        'O' => shopts.push((enabled, args.next().map(utf8).transpose()?)),
                        'o' => match args.next().as_deref().and_then(|a| a.to_str()) {
                            Some("braceexpand") => braceexpand = enabled,
                            Some("pipefail") => pipefail = enabled,
                            Some("errexit") => errexit = enabled,
                            Some("verbose") => verbose = enabled,
                            Some("errtrace") => errtrace = enabled,
                            Some("functrace") => functrace = enabled,
                            Some("nounset") => nounset = enabled,
                            Some("monitor") => monitor = Some(enabled),
                            Some("noclobber") => noclobber = enabled,
                            Some("noglob") => noglob = enabled,
                            _ => {
                                return Err(
                                    "expected braceexpand, pipefail, errexit, noglob or nounset after -o".into(),
                                );
                            }
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
        interactive |= terminal();
        Input::Stdin
    };
    let positional = args.map(utf8).collect::<Result<_, _>>()?;
    Ok(Action::Run(Invocation {
        input,
        arg0,
        positional,
        interactive,
        login,
        norc,
        noprofile,
        noediting,
        rcfile,
        check: check && !interactive,
        errexit,
        verbose,
        errtrace,
        functrace,
        nounset,
        noglob,
        noclobber,
        monitor,
        braceexpand,
        pipefail,
        shopts,
    }))
}
