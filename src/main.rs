mod execute;
mod parser;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use execute::Flow;

fn evaluate(program: &[parser::Pipeline], mut status: u8, replace_last: bool) -> Flow {
    for (index, pipeline) in program.iter().enumerate() {
        match execute::run(pipeline, status, replace_last && index + 1 == program.len()) {
            Ok(Flow::Continue(code)) => status = code,
            Ok(exit @ Flow::Exit(_)) => return exit,
            Err(e) => {
                eprintln!("tonari: {e}");
                status = 1;
            }
        }
    }
    Flow::Continue(status)
}

extern "C" fn interrupt(_: libc::c_int) {}

fn listen_for_interrupts() -> io::Result<()> {
    // A caught signal interrupts the terminal read and resets to its default
    // disposition on exec, so foreground programs still receive Ctrl-C.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = interrupt as *const () as usize;
    unsafe { libc::sigemptyset(&mut action.sa_mask) };
    for signal in [libc::SIGINT, libc::SIGQUIT] {
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn read_terminal_line(source: &mut Vec<u8>) -> io::Result<bool> {
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            return Ok(false);
        }
        source.extend_from_slice(&buf[..n as usize]);
        if source.ends_with(b"\n") {
            return Ok(true);
        }
    }
}

fn interactive() -> io::Result<u8> {
    listen_for_interrupts()?;
    let mut status = 0;
    let mut source = Vec::new();
    loop {
        let prompt = if source.is_empty() { "tonari$ " } else { "> " };
        let mut stderr = io::stderr().lock();
        stderr.write_all(prompt.as_bytes())?;
        stderr.flush()?;
        drop(stderr);
        let more = match read_terminal_line(&mut source) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                eprintln!();
                status = 130;
                source.clear();
                continue;
            }
            result => result?,
        };
        if !more && source.is_empty() {
            return Ok(status);
        }
        match std::str::from_utf8(&source) {
            Ok(text) => match parser::parse(text) {
                Ok(program) => match evaluate(&program, status, false) {
                    Flow::Continue(code) => status = code,
                    Flow::Exit(code) => return Ok(code),
                },
                Err(parser::Error::Incomplete(_)) if more => continue,
                Err(e) => {
                    eprintln!("tonari: {}", e.message());
                    status = 2;
                }
            },
            Err(_) => {
                eprintln!("tonari: expected UTF-8 command text");
                status = 2;
            }
        }
        if !more {
            return Ok(status);
        }
        source.clear();
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let arg = args.next();
    let code = match arg.as_deref().and_then(|s| s.to_str()) {
        Some("-c") => {
            let source = args.next();
            let Some(source) = source.as_deref().and_then(|s| s.to_str()) else {
                eprintln!("tonari: -c requires a UTF-8 command string");
                return ExitCode::from(2);
            };
            // Positional arguments to -c are reserved for parameter expansion.
            match parser::parse(source) {
                Ok(program) => match evaluate(&program, 0, true) {
                    Flow::Continue(code) | Flow::Exit(code) => code,
                },
                Err(e) => {
                    eprintln!("tonari: {}", e.message());
                    2
                }
            }
        }
        Some("--version") => {
            println!("tonari {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--help") => {
            println!("Usage: tonari [-c COMMAND | --version | --help]");
            0
        }
        None if arg.is_none() && io::stdin().is_terminal() => interactive().unwrap_or_else(|e| {
            eprintln!("tonari: {e}");
            1
        }),
        _ => {
            eprintln!("tonari: use -c COMMAND or start Tonari in a terminal");
            2
        }
    };
    ExitCode::from(code)
}
