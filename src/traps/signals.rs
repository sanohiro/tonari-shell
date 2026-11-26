use super::*;

const SIGNALS: &[(&str, i32)] = &[
    ("EXIT", 0),
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("ILL", libc::SIGILL),
    ("TRAP", libc::SIGTRAP),
    ("ABRT", libc::SIGABRT),
    ("BUS", libc::SIGBUS),
    #[cfg(target_os = "macos")]
    ("EMT", libc::SIGEMT),
    #[cfg(target_os = "macos")]
    ("INFO", libc::SIGINFO),
    ("FPE", libc::SIGFPE),
    ("KILL", libc::SIGKILL),
    ("USR1", libc::SIGUSR1),
    ("SEGV", libc::SIGSEGV),
    ("USR2", libc::SIGUSR2),
    ("PIPE", libc::SIGPIPE),
    ("ALRM", libc::SIGALRM),
    ("TERM", libc::SIGTERM),
    ("CHLD", libc::SIGCHLD),
    ("CONT", libc::SIGCONT),
    ("STOP", libc::SIGSTOP),
    ("TSTP", libc::SIGTSTP),
    ("TTIN", libc::SIGTTIN),
    ("TTOU", libc::SIGTTOU),
    ("URG", libc::SIGURG),
    ("XCPU", libc::SIGXCPU),
    ("XFSZ", libc::SIGXFSZ),
    ("VTALRM", libc::SIGVTALRM),
    ("PROF", libc::SIGPROF),
    ("WINCH", libc::SIGWINCH),
    ("IO", libc::SIGIO),
    ("SYS", libc::SIGSYS),
    #[cfg(target_os = "linux")]
    ("PWR", libc::SIGPWR),
    #[cfg(target_os = "linux")]
    ("STKFLT", libc::SIGSTKFLT),
];

pub(crate) fn signals() -> impl Iterator<Item = (&'static str, i32)> {
    let signals = SIGNALS.iter().copied();
    #[cfg(target_os = "linux")]
    {
        static REALTIME: std::sync::LazyLock<Vec<(String, i32)>> = std::sync::LazyLock::new(|| {
            let (low, high) = (libc::SIGRTMIN(), libc::SIGRTMAX());
            (low..=high)
                .map(|n| {
                    let name = if n == low {
                        "RTMIN".into()
                    } else if n == high {
                        "RTMAX".into()
                    } else if n - low <= high - n {
                        format!("RTMIN+{}", n - low)
                    } else {
                        format!("RTMAX-{}", high - n)
                    };
                    (name, n)
                })
                .collect()
        });
        signals.chain(
            std::iter::once_with(|| REALTIME.iter())
                .flatten()
                .map(|(name, n)| (name.as_str(), *n)),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        signals
    }
}

// Present the same numeric table to both signal-name builtins. Programs
// commonly extract the first column or parse several entries on one line.
pub(crate) fn print_signal_table(out: &mut impl Write) -> io::Result<()> {
    static TABLE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        use std::fmt::Write;
        let mut signals = signals().filter(|s| s.1 != 0).collect::<Vec<_>>();
        signals.sort_unstable_by_key(|s| s.1);
        let mut table = String::new();
        for row in signals.chunks(5) {
            for (i, (name, n)) in row.iter().enumerate() {
                if i != 0 {
                    table.push('\t');
                }
                write!(table, "{n:2}) SIG{name}").unwrap();
            }
            table.push('\n');
        }
        table
    });
    out.write_all(TABLE.as_bytes())
}

pub(crate) fn number(s: &str) -> Option<i32> {
    let number = s.parse::<i32>().ok();
    let s = s
        .get(..3)
        .filter(|prefix| prefix.eq_ignore_ascii_case("SIG"))
        .map_or(s, |_| &s[3..]);
    let known = signals()
        .find(|(name, n)| s.eq_ignore_ascii_case(name) || number == Some(*n))
        .map(|s| s.1);
    #[cfg(target_os = "linux")]
    if known.is_none()
        && let Some((prefix, offset)) = s.as_bytes().split_at_checked(6)
    {
        let (low, high) = (libc::SIGRTMIN(), libc::SIGRTMAX());
        let delta = std::str::from_utf8(offset).ok()?.parse::<u32>().ok()?;
        if delta > (high - low) as u32 {
            return None;
        }
        if prefix.eq_ignore_ascii_case(b"RTMIN+") {
            return Some(low + delta as i32);
        }
        if prefix.eq_ignore_ascii_case(b"RTMAX-") {
            return Some(high - delta as i32);
        }
    }
    known
}
