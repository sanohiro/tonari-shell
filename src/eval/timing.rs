use crate::Session;
use std::fmt::Write;
use std::time::{Duration, Instant};

fn cpu_times() -> (Duration, Duration) {
    let mut user = Duration::ZERO;
    let mut system = Duration::ZERO;
    for who in [libc::RUSAGE_SELF, libc::RUSAGE_CHILDREN] {
        let mut usage = std::mem::MaybeUninit::uninit();
        if unsafe { libc::getrusage(who, usage.as_mut_ptr()) } == 0 {
            let usage = unsafe { usage.assume_init() };
            let duration =
                |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
            user += duration(usage.ru_utime);
            system += duration(usage.ru_stime);
        }
    }
    (user, system)
}

pub(super) struct Timer {
    start: Instant,
    user: Duration,
    system: Duration,
}

impl Timer {
    pub(super) fn start() -> Self {
        let (user, system) = cpu_times();
        Self {
            start: Instant::now(),
            user,
            system,
        }
    }

    pub(super) fn report(self, session: &Session, posix: bool) {
        let real = self.start.elapsed();
        let (user, system) = cpu_times();
        let user = user.saturating_sub(self.user);
        let system = system.saturating_sub(self.system);
        let configured;
        let format = if posix {
            "real %2R\nuser %2U\nsys %2S"
        } else {
            configured = session.get("TIMEFORMAT");
            configured
                .as_deref()
                .unwrap_or("\nreal\t%3lR\nuser\t%3lU\nsys\t%3lS")
        };
        if format.is_empty() {
            return;
        }
        let mut out = String::new();
        let mut chars = format.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            let mut code = chars.next().unwrap_or('\0');
            if code == '%' {
                out.push('%');
                continue;
            }
            if code == 'P' {
                let elapsed = real.as_nanos();
                let scaled = (user + system).as_nanos() * 10_000;
                let hundredths = scaled.checked_div(elapsed).map_or(0, |whole| {
                    let twice = 2 * (scaled % elapsed);
                    whole + u128::from(twice > elapsed || twice == elapsed && whole % 2 == 1)
                });
                let _ = write!(out, "{}.{:02}", hundredths / 100, hundredths % 100);
                continue;
            }
            let mut precision = 3;
            if let Some(n) = code.to_digit(10) {
                precision = n.min(3) as usize;
                code = chars.next().unwrap_or('\0');
            }
            let long = code == 'l';
            if long {
                code = chars.next().unwrap_or('\0');
            }
            let duration = match code {
                'R' => real,
                'U' => user,
                'S' => system,
                _ => {
                    diagnostic!("TIMEFORMAT: invalid format character '{code}'");
                    return;
                }
            };
            // Round to the precision of the reference clock before truncating digits.
            let millis = (duration.as_micros() + 500) / 1000;
            let seconds = millis / 1000;
            if long {
                let _ = write!(out, "{}m{}", seconds / 60, seconds % 60);
            } else {
                let _ = write!(out, "{seconds}");
            }
            if precision != 0 {
                let fraction = millis % 1000 / 10_u128.pow(3 - precision as u32);
                let _ = write!(out, ".{fraction:0precision$}");
            }
            if long {
                out.push('s');
            }
        }
        eprintln!("{out}");
    }
}
