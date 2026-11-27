use super::{Action, Binding, Keymap, Session, next, previous};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::os::fd::RawFd;

pub(super) struct Terminal {
    original: libc::termios,
    bracketed_paste: bool,
}

impl Terminal {
    pub(super) fn enter(bracketed: bool) -> io::Result<Self> {
        let mut old = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(0, &mut old) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut new = old;
        new.c_lflag &= !(libc::ICANON | libc::ECHO | libc::IEXTEN);
        new.c_iflag &= !(libc::IXON | libc::ICRNL);
        new.c_cc[libc::VSUSP] = libc::_POSIX_VDISABLE as _;
        new.c_cc[libc::VMIN] = 0;
        new.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &new) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let term = Self {
            original: old,
            bracketed_paste: bracketed,
        };
        if bracketed {
            io::stderr().write_all(b"\x1b[?2004h")?;
        }
        Ok(term)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        if self.bracketed_paste {
            let _ = io::stderr().write_all(b"\x1b[?2004l");
        }
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.original);
        }
    }
}

fn byte() -> io::Result<Option<u8>> {
    let mut key = [0u8];
    loop {
        match crate::terminal_read(&mut key, true) {
            Ok(n) => return Ok((n == 1).then_some(key[0])),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e),
        }
    }
}

pub(crate) fn ready(fd: RawFd, ms: i32) -> bool {
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut p, 1, ms) > 0 }
}

#[derive(Default)]
pub(super) struct Keys {
    pub(super) queued: VecDeque<u8>,
    expansions: usize,
    sequence: Vec<u8>,
}

impl Keys {
    // Bound one frame of committed text. Stop at control keys even within this budget.
    pub(super) const BURST_BYTES: usize = 4096;
    pub(super) const MAX_QUEUED_BYTES: usize = 64 * 1024;
    const MAX_MACRO_EXPANSIONS: usize = 64;

    pub(super) fn new(queued: VecDeque<u8>) -> Self {
        Self {
            queued,
            ..Self::default()
        }
    }

    pub(super) fn can_replay(&self, session: &Session) -> bool {
        self.queued.front().is_some_and(|&b| {
            let width = match b {
                8 | 32..=127 => 1,
                _ => match multibyte_width(b) {
                    Some(width) => width,
                    None => return false,
                },
            };
            self.queued.len() >= width && !session.bindings.custom_start(b)
        })
    }

    pub(super) fn expand_macro(&mut self, seq: Vec<u8>) -> io::Result<()> {
        self.expansions += 1;
        if self.expansions > Self::MAX_MACRO_EXPANSIONS
            || self.queued.len().saturating_add(seq.len()) > Self::MAX_QUEUED_BYTES
        {
            self.queued.clear();
            io::stderr().write_all(b"\x07")?;
        } else {
            for key in seq.into_iter().rev() {
                self.queued.push_front(key);
            }
        }
        Ok(())
    }

    pub(super) fn input_pending(&self) -> bool {
        !self.queued.is_empty()
            || ready(0, 0)
            || crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub(super) fn get(&mut self) -> io::Result<Option<u8>> {
        if let Some(key) = self.queued.pop_front() {
            return Ok(Some(key));
        }
        self.expansions = 0;
        byte()
    }

    // VMIN and VTIME are zero. Only consume bytes belonging to this edit:
    // a foreground child can read the terminal immediately after accept-line.
    fn immediate(&mut self) -> Option<u8> {
        if let Some(b) = self.queued.pop_front() {
            return Some(b);
        }
        let mut b = 0u8;
        if unsafe { libc::read(0, std::ptr::addr_of_mut!(b).cast(), 1) } == 1 {
            self.expansions = 0;
            Some(b)
        } else {
            None
        }
    }

    // A terminal repeat or macro may deliver several completed motions at once.
    // Stop before a partial escape, custom binding, or suggestion-accepting Right.
    pub(super) fn motion_burst(
        &mut self,
        text: &str,
        mut cursor: usize,
        session: &Session,
    ) -> usize {
        if session.bindings.map != Keymap::Emacs {
            return cursor;
        }
        for _ in 0..64 {
            let Some(first) = self.immediate() else { break };
            let mut seq = [first, 0, 0];
            let mut len = 1;
            if first == 27
                && !session.bindings.custom_start(first)
                && let Some(b) = self.immediate()
            {
                seq[1] = b;
                len = 2;
                if matches!(b, b'[' | b'O')
                    && let Some(b) = self.immediate()
                {
                    seq[2] = b;
                    len = 3;
                }
            }
            match &seq[..len] {
                [2] if !session.bindings.custom_start(2) => cursor = previous(text, cursor),
                [6] if !session.bindings.custom_start(6) => cursor = next(text, cursor),
                [27, b'[' | b'O', b'D'] => cursor = previous(text, cursor),
                [27, b'[' | b'O', b'C'] if cursor < text.len() => cursor = next(text, cursor),
                other => {
                    for &b in other.iter().rev() {
                        self.queued.push_front(b);
                    }
                    break;
                }
            }
        }
        cursor
    }

    pub(super) fn character(&mut self, first: u8) -> io::Result<Option<char>> {
        if first.is_ascii() {
            return Ok(Some(char::from(first)));
        }
        let Some(width) = multibyte_width(first) else {
            return Ok(None);
        };
        let mut bytes = [first; 4];
        for slot in &mut bytes[1..width] {
            let b = if let Some(b) = self.immediate() {
                b
            } else if let Some(b) = self.get()? {
                b
            } else {
                return Ok(None);
            };
            if b & 0xc0 != 0x80 {
                self.queued.push_front(b);
                return Ok(None);
            }
            *slot = b;
        }
        Ok(std::str::from_utf8(&bytes[..width])
            .ok()
            .and_then(|s| s.chars().next()))
    }

    pub(super) fn append_burst(
        &mut self,
        first: u8,
        session: &Session,
        out: &mut String,
    ) -> io::Result<()> {
        let Some(c) = self.character(first)? else {
            return Ok(());
        };
        out.push(c);
        if session.bindings.map == Keymap::Command {
            return Ok(());
        }
        // Gather an immediately available run without waiting for more keys.
        // A bound keeps long streams responsive to signals and screen updates.
        let mut bytes = [0; Self::BURST_BYTES];
        let mut n = 0;
        while n < bytes.len() {
            let Some(b) = self.immediate() else { break };
            if !session.bindings.literal_byte(b) {
                self.queued.push_front(b);
                break;
            }
            bytes[n] = b;
            n += 1;
        }
        let mut rest = &bytes[..n];
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    out.push_str(text);
                    break;
                }
                Err(e) => {
                    let (valid, tail) = rest.split_at(e.valid_up_to());
                    out.push_str(std::str::from_utf8(valid).unwrap());
                    if let Some(invalid) = e.error_len() {
                        rest = &tail[invalid..];
                    } else {
                        // Display the valid prefix before waiting for a split scalar.
                        for &b in tail.iter().rev() {
                            self.queued.push_front(b);
                        }
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn binding(&mut self, first: u8, session: &Session) -> io::Result<(Binding, bool)> {
        if unbound_text(first, session) {
            return Ok((Binding::Edit(Action::Insert), false));
        }
        let fallback = if session.bindings.map != Keymap::Command && first >= 32 && first != 127 {
            Binding::Edit(Action::Insert)
        } else {
            Binding::Removed
        };
        let (exact, mut longer) = session.bindings.lookup(&[first]);
        if !longer {
            return Ok((exact.unwrap_or(fallback), false));
        }
        let mut seq = std::mem::take(&mut self.sequence);
        seq.clear();
        seq.push(first);
        let mut best = exact.map(|action| (1, action));
        while longer {
            // Consume one byte at a time; a foreground child owns later typeahead.
            let key = if let Some(key) = self.immediate() {
                key
            } else if ready(0, session.bindings.timeout()) {
                let Some(key) = self.get()? else { break };
                key
            } else {
                break;
            };
            seq.push(key);
            let (exact, prefix) = session.bindings.lookup(&seq);
            if let Some(action) = exact {
                best = Some((seq.len(), action));
            }
            longer = prefix;
        }
        let (len, action) = best.unwrap_or((1, fallback));
        // Preserve unmatched bytes when a custom sequence shares a prefix with ordinary typing.
        for &key in seq[len..].iter().rev() {
            self.queued.push_front(key);
        }
        let right = matches!(&seq[..len], b"\x1b[C" | b"\x1bOC");
        self.sequence = seq;
        Ok((action, right))
    }

    // After the end marker one byte still belongs to the editor. It could be
    // Enter, so leave everything after that byte on the tty for the next reader.
    pub(super) fn paste(
        &mut self,
        mut paint: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<String> {
        const END: &[u8; 6] = b"\x1b[201~";
        let mut data = Vec::new();
        let mut matched = 0;
        let mut chunk = [0; END.len() + 1];
        let mut displayed = 0;
        const PREVIEW_INTERVAL: usize = 16 * 1024;
        let mut checkpoint = PREVIEW_INTERVAL;
        let mut painted_at = std::time::Instant::now();
        'paste: loop {
            if crate::INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed)
                || crate::traps::input_interrupted()
            {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let available = data.len() - matched;
            if available >= checkpoint {
                checkpoint = available + PREVIEW_INTERVAL;
                if painted_at.elapsed() >= std::time::Duration::from_millis(16) {
                    paint(&data[..available])?;
                    displayed = available;
                    painted_at = std::time::Instant::now();
                }
            }
            let limit = chunk.len() - matched;
            let n = if self.queued.is_empty() {
                let immediate = unsafe { libc::read(0, chunk.as_mut_ptr().cast(), limit) };
                if immediate > 0 {
                    immediate as usize
                } else {
                    // A split packet may pause for an arbitrary time. Give a
                    // contiguous paste one short frame interval to resume.
                    if available > displayed && !ready(0, 8) {
                        paint(&data[..available])?;
                        displayed = available;
                        painted_at = std::time::Instant::now();
                    }
                    match crate::terminal_read(&mut chunk[..limit], true) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                        Err(e) => return Err(e),
                    }
                }
            } else {
                let n = limit.min(self.queued.len());
                for slot in &mut chunk[..n] {
                    *slot = self.queued.pop_front().unwrap();
                }
                n
            };
            data.extend_from_slice(&chunk[..n]);
            if matched == 0 && !chunk[..n].contains(&END[0]) {
                continue;
            }
            for (i, &b) in chunk[..n].iter().enumerate() {
                matched = if b == END[matched] {
                    matched + 1
                } else {
                    usize::from(b == END[0])
                };
                if matched == END.len() {
                    let following = &chunk[i + 1..n];
                    if let Some(&key) = following.first() {
                        self.queued.push_front(key);
                    }
                    data.truncate(data.len() - END.len() - following.len());
                    break 'paste;
                }
            }
        }
        if memchr::memchr(b'\r', &data).is_some() {
            let mut after_cr = false;
            data.retain_mut(|b| {
                let keep = !(after_cr && *b == b'\n');
                after_cr = *b == b'\r';
                if after_cr {
                    *b = b'\n';
                }
                keep
            });
        }
        Ok(String::from_utf8(data)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
    }
}
fn multibyte_width(first: u8) -> Option<usize> {
    match first {
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

pub(super) fn unbound_text(first: u8, session: &Session) -> bool {
    session.bindings.map != Keymap::Command && session.bindings.literal_byte(first)
}
