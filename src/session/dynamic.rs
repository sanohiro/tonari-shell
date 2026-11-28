// Dynamic Bash integers. An ordinary local or an unset tombstone can shadow
// the inherited binding; no entropy or timer state is initialized at startup.
use super::Session;
use std::cell::Cell;

#[derive(Clone, Copy, Default)]
struct Random {
    seed: u32,
    last: u16,
    initialized: bool,
}

#[derive(Clone, Default)]
pub(super) struct Dynamic {
    seconds: Cell<Option<i64>>,
    random: Cell<Random>,
    listed: [Cell<Option<i64>>; 5],
    pub(super) lineno: Cell<Option<usize>>,
    pub(super) pid: Cell<Option<u32>>,
    pub(super) depth: Cell<Option<u32>>,
    text: std::cell::RefCell<Vec<(&'static str, String)>>,
    pub(super) groups_read: Cell<bool>,
    pub(super) stack_read: Cell<bool>,
}

fn number_slot(name: &str) -> Option<usize> {
    Some(match name {
        "SECONDS" => 0,
        "RANDOM" => 1,
        "SRANDOM" => 2,
        "HISTCMD" => 3,
        "BASH_MONOSECONDS" => 4,
        _ => return None,
    })
}

pub(super) fn is_dynamic(name: &str) -> bool {
    number_slot(name).is_some()
}

pub(super) fn integer_attribute(name: &str) -> bool {
    matches!(name, "SECONDS" | "RANDOM" | "SRANDOM" | "OPTIND")
}

fn entropy() -> Option<u32> {
    // Darwin and musl both export getentropy. The libc crate only declares it
    // for some of the supported targets.
    unsafe extern "C" {
        fn getentropy(buf: *mut libc::c_void, len: libc::size_t) -> libc::c_int;
    }
    let mut bytes = [0u8; 4];
    (unsafe { getentropy(bytes.as_mut_ptr().cast(), bytes.len()) } == 0)
        .then(|| u32::from_ne_bytes(bytes))
}

impl Session {
    pub(crate) fn enter_subshell(&mut self) {
        self.subshell_depth += 1;
        if self.shopt.compat > 44 {
            self.parent_loop_in_subshell |= self.loop_depth != 0;
            self.loop_depth = 0;
        }
        self.dynamic.random.set(Random::default());
    }

    pub(super) fn observe_text(&self, name: &'static str, value: String) -> String {
        let mut entries = self.dynamic.text.borrow_mut();
        if let Some((_, old)) = entries.iter_mut().find(|(key, _)| *key == name) {
            value.clone_into(old);
        } else {
            entries.push((name, value.clone()));
        }
        value
    }

    pub(super) fn listed_text(&self, name: &str) -> Option<String> {
        match name {
            "LINENO" => self.dynamic.lineno.get().map(|n| n.to_string()),
            "BASHPID" => self.dynamic.pid.get().map(|n| n.to_string()),
            "BASH_SUBSHELL" => self.dynamic.depth.get().map(|n| n.to_string()),
            _ => self
                .dynamic
                .text
                .borrow()
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone()),
        }
    }

    pub(super) fn listed_number(&self, name: &str) -> Option<i64> {
        self.dynamic.listed[number_slot(name)?].get()
    }

    pub(super) fn dynamic_number(&self, name: &str) -> Option<i64> {
        let value = match name {
            "SECONDS" => {
                let origin = self.dynamic.seconds.get().unwrap_or_else(|| {
                    let initial = std::env::var("SECONDS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let origin = self.started_at.wrapping_sub(initial);
                    self.dynamic.seconds.set(Some(origin));
                    origin
                });
                Some(unsafe { libc::time(std::ptr::null_mut()) }.wrapping_sub(origin))
            }
            "RANDOM" => {
                let mut rng = self.dynamic.random.get();
                if !rng.initialized {
                    rng.seed = entropy()?;
                    rng.initialized = true;
                }
                loop {
                    const MODULUS: i64 = 2_147_483_647;
                    let seed = if rng.seed == 0 { 123_459_876 } else { rng.seed };
                    let value =
                        16_807 * i64::from(seed % 127_773) - 2_836 * i64::from(seed / 127_773);
                    rng.seed = if value < 0 { value + MODULUS } else { value } as u32;
                    let bits = if self.shopt.compat <= 50 {
                        rng.seed
                    } else {
                        rng.seed ^ (rng.seed >> 16)
                    };
                    let next = (bits & 0x7fff) as u16;
                    if next != rng.last {
                        rng.last = next;
                        self.dynamic.random.set(rng);
                        break Some(i64::from(next));
                    }
                }
            }
            "HISTCMD" => Some(if self.history_enabled {
                self.history_command_number() as i64
            } else {
                0
            }),
            "BASH_MONOSECONDS" => {
                let mut ts = std::mem::MaybeUninit::uninit();
                Some(
                    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, ts.as_mut_ptr()) } == 0 {
                        unsafe { ts.assume_init() }.tv_sec
                    } else {
                        unsafe { libc::time(std::ptr::null_mut()) }
                    },
                )
            }
            "SRANDOM" => entropy().map(i64::from),
            _ => return None,
        }?;
        self.dynamic.listed[number_slot(name)?].set(Some(value));
        Some(value)
    }

    pub(super) fn set_dynamic(&self, name: &str, value: &str) {
        let value = value.parse::<i64>().unwrap_or(0);
        if let Some(slot) = number_slot(name) {
            self.dynamic.listed[slot].set(Some(value));
        }
        match name {
            "SECONDS" => self.dynamic.seconds.set(Some(
                unsafe { libc::time(std::ptr::null_mut()) }.wrapping_sub(value),
            )),
            "RANDOM" => self.dynamic.random.set(Random {
                seed: value as u32,
                initialized: true,
                last: 0,
            }),
            _ => {}
        }
    }
}
