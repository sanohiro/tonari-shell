use std::io::{self, Write};

#[cfg(all(target_os = "linux", target_env = "gnu"))]
type Resource = libc::__rlimit_resource_t;
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
type Resource = libc::c_int;

struct Limit {
    flag: char,
    resource: Option<Resource>,
    unit: libc::rlim_t,
    name: &'static str,
    units: &'static str,
}

const fn limit(
    flag: char,
    resource: Resource,
    unit: libc::rlim_t,
    name: &'static str,
    units: &'static str,
) -> Limit {
    Limit {
        flag,
        resource: Some(resource),
        unit,
        name,
        units,
    }
}

const LIMITS: &[Limit] = &[
    #[cfg(target_os = "linux")]
    limit(
        'R',
        libc::RLIMIT_RTTIME,
        1,
        "real-time non-blocking time",
        "microseconds",
    ),
    limit('c', libc::RLIMIT_CORE, 1024, "core file size", "blocks"),
    limit('d', libc::RLIMIT_DATA, 1024, "data seg size", "kbytes"),
    #[cfg(target_os = "linux")]
    limit('e', libc::RLIMIT_NICE, 1, "scheduling priority", ""),
    limit('f', libc::RLIMIT_FSIZE, 1024, "file size", "blocks"),
    #[cfg(target_os = "linux")]
    limit('i', libc::RLIMIT_SIGPENDING, 1, "pending signals", ""),
    limit(
        'l',
        libc::RLIMIT_MEMLOCK,
        1024,
        "max locked memory",
        "kbytes",
    ),
    limit('m', libc::RLIMIT_RSS, 1024, "max memory size", "kbytes"),
    limit('n', libc::RLIMIT_NOFILE, 1, "open files", ""),
    Limit {
        flag: 'p',
        resource: None,
        unit: 512,
        name: "pipe size",
        units: "512 bytes",
    },
    #[cfg(target_os = "linux")]
    limit(
        'q',
        libc::RLIMIT_MSGQUEUE,
        1,
        "POSIX message queues",
        "bytes",
    ),
    #[cfg(target_os = "linux")]
    limit('r', libc::RLIMIT_RTPRIO, 1, "real-time priority", ""),
    limit('s', libc::RLIMIT_STACK, 1024, "stack size", "kbytes"),
    limit('t', libc::RLIMIT_CPU, 1, "cpu time", "seconds"),
    limit('u', libc::RLIMIT_NPROC, 1, "max user processes", ""),
    limit('v', libc::RLIMIT_AS, 1024, "virtual memory", "kbytes"),
    #[cfg(target_os = "linux")]
    limit('x', libc::RLIMIT_LOCKS, 1, "file locks", ""),
];

impl Limit {
    fn current(&self) -> io::Result<libc::rlimit> {
        let mut values = libc::rlimit {
            rlim_cur: libc::PIPE_BUF as libc::rlim_t,
            rlim_max: libc::PIPE_BUF as libc::rlim_t,
        };
        if let Some(resource) = self.resource
            && unsafe { libc::getrlimit(resource, &mut values) } == -1
        {
            return Err(io::Error::last_os_error());
        }
        Ok(values)
    }

    fn report(
        &self,
        out: &mut dyn Write,
        values: &libc::rlimit,
        hard: bool,
        label: bool,
    ) -> io::Result<()> {
        if label {
            // Bash left-aligns the name and right-aligns the unit/flag field.
            let blanks = b"                    ";
            out.write_all(self.name.as_bytes())?;
            out.write_all(&blanks[..20usize.saturating_sub(self.name.len())])?;
            let suffix_len = if self.units.is_empty() {
                5
            } else {
                7 + self.units.len()
            };
            out.write_all(b" ")?;
            out.write_all(&blanks[..20usize.saturating_sub(suffix_len)])?;
            out.write_all(b"(")?;
            if !self.units.is_empty() {
                out.write_all(self.units.as_bytes())?;
                out.write_all(b", ")?;
            }
            out.write_all(&[b'-', self.flag as u8, b')', b' '])?;
        }
        let n = if hard {
            values.rlim_max
        } else {
            values.rlim_cur
        };
        if n == libc::RLIM_INFINITY {
            writeln!(out, "unlimited")
        } else {
            writeln!(out, "{}", n / self.unit)
        }
    }

    fn set(&self, mut values: libc::rlimit, arg: &str, soft: bool, hard: bool) -> io::Result<()> {
        let mut new = match arg {
            "soft" => values.rlim_cur,
            "hard" => values.rlim_max,
            "unlimited" => libc::RLIM_INFINITY,
            _ => arg
                .parse::<libc::rlim_t>()
                .ok()
                .filter(|_| arg.bytes().all(|c| c.is_ascii_digit()))
                .and_then(|n| n.checked_mul(self.unit))
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, format!("{arg}: invalid limit"))
                })?,
        };
        let resource = self
            .resource
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
        if soft && !hard && new == libc::RLIM_INFINITY && unsafe { libc::geteuid() } != 0 {
            new = values.rlim_max;
        }
        if soft {
            values.rlim_cur = new;
        }
        if hard {
            values.rlim_max = new;
        }
        if unsafe { libc::setrlimit(resource, &values) } == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

pub fn run(args: &[String], out: &mut dyn Write) -> u8 {
    let mut args = args.iter().map(String::as_str).peekable();
    let (mut soft, mut hard, mut all) = (false, false, false);
    let mut requests = Vec::new();
    while let Some(opt) = args.next_if(|s| s.len() > 1 && s.starts_with('-')) {
        if opt == "--" {
            break;
        }
        for (i, flag) in opt[1..].char_indices() {
            match flag {
                'S' => soft = true,
                'H' => hard = true,
                'a' => all = true,
                _ => {
                    let Some(limit) = LIMITS.iter().find(|item| item.flag == flag) else {
                        eprintln!("tonari: ulimit: -{flag}: invalid option");
                        return 2;
                    };
                    let rest = &opt[1 + i + flag.len_utf8()..];
                    let value = if rest.is_empty() {
                        args.next_if(|s| !s.starts_with('-'))
                    } else {
                        Some(rest)
                    };
                    requests.push((limit, value));
                    break;
                }
            }
        }
    }
    if all {
        let mut lines = Vec::with_capacity(LIMITS.len() * 60);
        for limit in LIMITS {
            if let Ok(values) = limit.current()
                && limit
                    .report(&mut lines, &values, hard && !soft, true)
                    .is_err()
            {
                return 1;
            }
        }
        if out.write_all(&lines).is_err() {
            return 1;
        }
    } else {
        if let Some((_, arg)) = requests.last_mut() {
            if arg.is_none() {
                *arg = args.next();
            }
        } else {
            let file = LIMITS.iter().find(|limit| limit.flag == 'f').unwrap();
            requests.push((file, args.next()));
        }
        let label = requests.len() > 1;
        for (limit, arg) in requests {
            let result = limit.current().and_then(|values| match arg {
                Some(arg) => limit.set(values, arg, soft || !hard, hard || !soft),
                None => limit.report(out, &values, hard && !soft, label),
            });
            if let Err(error) = result {
                eprintln!("tonari: ulimit: {}: {error}", limit.name);
                return 1;
            }
        }
    }
    u8::from(out.flush().is_err())
}
