// The interpreter stays on its own thread. Only the display is speculative:
// replay the exact keys against the completed line when Tab returns.
use super::{Action, Binding, Keys, Screen, bindings::Bindings, rubout};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

struct Preview {
    pid: libc::pid_t,
    prompt: String,
    line: String,
    cursor: usize,
    screen: Screen,
    bindings: Bindings,
    overwrite: bool,
    replay: VecDeque<u8>,
    utf8: Vec<u8>,
    stopped: bool,
}

thread_local! {
    static STEPS: Cell<u8> = const { Cell::new(0) };
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static CURRENT: RefCell<Option<Preview>> = const { RefCell::new(None) };
}

pub(super) fn during<T>(
    view: super::LineView<'_>,
    screen: &mut Screen,
    keys: &mut Keys,
    bindings: Bindings,
    overwrite: bool,
    run: impl FnOnce() -> T,
) -> T {
    // A macro or a nested read -e already owns the pending input.
    if !keys.queued.is_empty()
        || CURRENT.with_borrow(Option::is_some)
        || bindings.map == super::Keymap::Command
    {
        return run();
    }
    let pending = Preview {
        pid: unsafe { libc::getpid() },
        prompt: view.prompt.into(),
        line: view.text.into(),
        cursor: view.cursor,
        screen: Screen {
            output: String::new(),
            enable_paste: false,
            row: screen.row,
            width: screen.width,
            height: screen.height,
            view: Default::default(),
            column: screen.column,
            had_hint: false,
            tail: screen.tail,
        },
        replay: VecDeque::new(),
        utf8: Vec::new(),
        bindings,
        overwrite,
        stopped: false,
    };
    CURRENT.with_borrow_mut(|slot| *slot = Some(pending));
    let result = run();
    let preview = CURRENT.with_borrow_mut(Option::take).unwrap();
    screen.row = preview.screen.row;
    keys.queued = preview.replay;
    result
}

// Arbitrary Bash code may read the terminal, even after a helper has exited.
// Only callbacks that give the editor ownership of typeahead may consume it.
pub(crate) fn with_input<T>(enabled: bool, run: impl FnOnce() -> T) -> T {
    // The editor uses VMIN=0 for bounded read-ahead. An inherited read(2)
    // in a Bash callback must block for a byte instead of observing EOF.
    let mut saved = unsafe { std::mem::zeroed::<libc::termios>() };
    let restore = !enabled
        && active()
        && terminal_available()
        && unsafe { libc::tcgetattr(0, &mut saved) } == 0
        && saved.c_cc[libc::VMIN] == 0
        && {
            let mut blocking = saved;
            blocking.c_cc[libc::VMIN] = 1;
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &blocking) == 0 }
        };
    let previous = ENABLED.replace(enabled && ENABLED.get());
    let result = run();
    ENABLED.set(previous);
    if restore {
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &saved);
        }
    }
    result
}

pub(crate) fn active() -> bool {
    ENABLED.get()
        && CURRENT.with_borrow(|p| {
            p.as_ref()
                .is_some_and(|p| p.pid == unsafe { libc::getpid() })
        })
}

pub(crate) fn wants_keys() -> bool {
    ENABLED.get()
        && CURRENT.with_borrow(|slot| {
            slot.as_ref()
                .is_some_and(|p| !p.stopped && p.pid == unsafe { libc::getpid() })
        })
}

// A function can redirect stdin or stderr around a nested command.
fn terminal_available() -> bool {
    let group = unsafe { libc::getpgrp() };
    unsafe { libc::tcgetpgrp(0) == group && libc::tcgetpgrp(2) == group }
}

pub(crate) fn can_wait() -> bool {
    wants_keys() && terminal_available()
}

// Capture helpers may produce more than a pipeful. Prefer draining output
// when both descriptors are ready and continue through EOF before waitpid.
pub(crate) fn read_capture<R: std::io::Read + std::os::fd::AsRawFd>(
    reader: &mut R,
    output: &mut Vec<u8>,
) -> std::io::Result<()> {
    let mut buf = [0u8; 8192];
    while wants_keys() {
        let mut fds = [
            libc::pollfd {
                fd: reader.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if fds[0].revents != 0 {
            match reader.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => output.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        if fds[1].revents != 0 {
            tty_event(fds[1].revents);
        }
    }
    reader.read_to_end(output).map(|_| ())
}

pub(crate) fn statement() {
    if ENABLED.get() {
        let step = STEPS.get().wrapping_add(1);
        STEPS.set(step);
        if step == 0 {
            poll();
        }
    }
}

// Native scans already checked that this process owns a preview. Asking the
// tty for its queued byte count avoids registering a poll waiter when idle.
pub(crate) fn scan_input() {
    let mut queued: libc::c_int = 0;
    if unsafe { libc::ioctl(0, libc::FIONREAD, &mut queued) } < 0 || queued > 0 {
        poll();
    }
}

pub(crate) fn poll() {
    if wants_keys() {
        let mut fd = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut fd, 1, 0) } > 0 {
            tty_event(fd.revents);
        }
    }
}

fn tty_event(events: libc::c_short) {
    CURRENT.with_borrow_mut(|slot| {
        let Some(preview) = slot else { return };
        if events & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 || !terminal_available() {
            preview.stopped = true;
        } else if events & libc::POLLIN != 0 {
            preview.typeahead();
        }
    });
}

impl Preview {
    fn insert_text(&mut self, text: &str) {
        let end = super::overwrite_end(&self.line, self.cursor, text, self.overwrite);
        self.line.replace_range(self.cursor..end, text);
        self.cursor += text.len();
    }

    fn typeahead(&mut self) {
        let append = self.cursor == self.line.len() && self.cursor >= super::VIEWPORT_THRESHOLD;
        let start = self.cursor;
        let mut erased = false;
        let mut edited = false;
        let mut text = String::new();
        for n in 0..Keys::BURST_BYTES {
            let mut byte = 0u8;
            if unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) } != 1 {
                // VMIN=0. A hung-up terminal stays readable.
                if n == 0 {
                    self.stopped = true;
                }
                break;
            }
            self.replay.push_back(byte);
            if self.replay.len() >= Keys::MAX_QUEUED_BYTES {
                self.stopped = true;
                break;
            }
            if self.utf8.is_empty() && !self.bindings.literal_byte(byte) {
                let (action, longer) = self.bindings.lookup(&[byte]);
                if !longer && matches!(action, Some(Binding::Edit(Action::Rubout))) {
                    self.insert_text(&text);
                    text.clear();
                    erased = true;
                    self.cursor = rubout(&mut self.line, self.cursor, self.overwrite, None);
                    edited = true;
                    continue;
                }
                // Enter may launch an external reader. Leave everything after it in the tty.
                self.stopped = true;
                break;
            }
            if byte.is_ascii() && self.utf8.is_empty() {
                text.push(char::from(byte));
                edited = true;
                continue;
            }
            self.utf8.push(byte);
            match std::str::from_utf8(&self.utf8) {
                Ok(scalar) => {
                    text.push_str(scalar);
                    self.utf8.clear();
                    edited = true;
                }
                Err(e) if e.error_len().is_none() => {}
                Err(_) => {
                    self.stopped = true;
                    break;
                }
            }
        }
        self.insert_text(&text);
        if edited {
            let drawn = if append
                && !erased
                && self.screen.can_append(&self.line[start..])
                && let Some(pos) = self.screen.tail
            {
                self.screen.append(&self.line[start..], pos)
            } else {
                self.screen.draw(&self.prompt, &self.line, self.cursor, "")
            };
            if drawn.is_err() {
                self.stopped = true;
            }
        }
    }
}

// Wait for a completion helper without holding up the display. The caller
// transfers the tty normally if the child stops to request terminal access.
pub(crate) fn wait_child(pid: libc::pid_t) -> std::io::Result<i32> {
    let mask = crate::traps::WaitMask::for_children(0)?;
    let mut signaled = false;
    loop {
        let mut status = 0;
        let got = crate::traps::waitpid(pid, &mut status, libc::WUNTRACED | libc::WNOHANG);
        if got == pid {
            return Ok(status);
        }
        if got < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if !signaled && (crate::INTERRUPTED.load(Ordering::Relaxed) || crate::traps::hanging_up()) {
            let sig = if crate::traps::hanging_up() {
                libc::SIGHUP
            } else {
                libc::SIGINT
            };
            unsafe {
                libc::kill(-pid, sig);
            }
            signaled = true;
        }
        if wants_keys() && !signaled {
            if mask.input_or_child(None)? {
                poll();
            }
        } else {
            mask.suspend();
        }
    }
}
