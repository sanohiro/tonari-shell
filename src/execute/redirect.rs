use super::*;

pub(super) enum Action<'a> {
    Open(RawFd, &'a str, i32),
    HereInput(RawFd, &'a [u8]),
    Dup(RawFd, RawFd),
    Close(RawFd),
    CloseMoved(RawFd),
}

pub(super) fn action(r: &Redirect) -> Action<'_> {
    match r.target {
        Target::CloseMoved => Action::CloseMoved(r.fd),
        Target::Duplicate if r.word == "-" => Action::Close(r.fd),
        Target::Duplicate => Action::Dup(r.word.parse().unwrap(), r.fd),
        Target::HereDoc { .. } | Target::HereString => Action::HereInput(r.fd, r.word.as_bytes()),
        _ => {
            let flags = match r.target {
                Target::Read => libc::O_RDONLY,
                Target::ReadWrite => libc::O_RDWR | libc::O_CREAT,
                Target::NoClobber => libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                Target::Clobber | Target::Write => libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                Target::Append => libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                Target::CloseMoved
                | Target::Duplicate
                | Target::OutputWord
                | Target::HereDoc { .. }
                | Target::HereString => unreachable!(),
            };
            Action::Open(r.fd, &r.word, flags)
        }
    }
}

pub(super) fn actions(cmd: &PreparedCommand) -> Vec<Action<'_>> {
    cmd.redirects.iter().map(action).collect()
}

pub(super) fn duplicate(src: RawFd, dest: RawFd) -> io::Result<()> {
    loop {
        if unsafe { libc::dup2(src, dest) } >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn connect_address(
    family: i32,
    kind: i32,
    protocol: i32,
    addr: *const libc::sockaddr,
    size: libc::socklen_t,
) -> io::Result<OwnedFd> {
    let fd = unsafe { libc::socket(family, kind, protocol) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::connect(fd, addr, size) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(socket)
}

// Resolve services as well as numeric ports, and try every address returned for
// the requested protocol. The open descriptor has the same inheritance as open(2).
fn network_redirect(path: &CStr, address: &[u8], kind: i32) -> Option<OwnedFd> {
    let Some(separator) = address.iter().position(|&b| b == b'/') else {
        write_error(path.to_bytes(), b": expected host/port");
        return None;
    };
    let host = &address[..separator];
    let service_start = path.to_bytes().len() - address.len() + separator + 1;
    let service = CStr::from_bytes_with_nul(&path.to_bytes_with_nul()[service_start..]).ok()?;
    if host.is_empty() || service.to_bytes().is_empty() {
        write_error(path.to_bytes(), b": expected host/port");
        return None;
    }
    // Numeric IPv4 datagrams need no resolver or service database. Other address
    // forms, including scoped IPv6, use the system resolver below.
    if kind == libc::SOCK_DGRAM
        && let Some((ip, port)) = std::str::from_utf8(host)
            .ok()
            .and_then(|s| s.parse::<std::net::Ipv4Addr>().ok())
            .zip(
                service
                    .to_str()
                    .ok()
                    .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|s| s.parse::<u16>().ok()),
            )
    {
        let mut dest: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        dest.sin_family = libc::AF_INET as _;
        dest.sin_port = port.to_be();
        dest.sin_addr.s_addr = u32::from_ne_bytes(ip.octets());
        let size = std::mem::size_of_val(&dest) as libc::socklen_t;
        #[cfg(target_os = "macos")]
        {
            dest.sin_len = size as u8;
        }
        return match connect_address(
            libc::AF_INET,
            kind,
            0,
            std::ptr::from_ref(&dest).cast(),
            size,
        ) {
            Ok(fd) => Some(fd),
            Err(error) => {
                write_error_detail(path.to_bytes(), b": ", error.to_string().as_bytes());
                None
            }
        };
    }
    let host = CString::new(host).ok()?;
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_socktype = kind;
    let mut list = std::ptr::null_mut();
    let result = unsafe { libc::getaddrinfo(host.as_ptr(), service.as_ptr(), &hints, &mut list) };
    if result != 0 {
        write_error_detail(
            path.to_bytes(),
            b": ",
            unsafe { CStr::from_ptr(libc::gai_strerror(result)) }.to_bytes(),
        );
        return None;
    }
    let mut current = list;
    let mut connected = None;
    let mut error = io::Error::from(io::ErrorKind::AddrNotAvailable);
    while let Some(entry) = unsafe { current.as_ref() } {
        match connect_address(
            entry.ai_family,
            kind,
            entry.ai_protocol,
            entry.ai_addr,
            entry.ai_addrlen,
        ) {
            Ok(fd) => {
                connected = Some(fd);
                break;
            }
            Err(e) => error = e,
        }
        current = entry.ai_next;
    }
    unsafe { libc::freeaddrinfo(list) };
    if connected.is_none() {
        write_error_detail(path.to_bytes(), b": ", error.to_string().as_bytes());
    }
    connected
}

// Only this standard device can be opened speculatively. Open without create
// or truncate, and inspect the descriptor before committing to the fast path.
// An installation with a missing or regular /dev/null uses normal redirection.
pub(super) fn open_null_redirect(redirects: &[Redirect]) -> Option<(RawFd, OwnedFd)> {
    use std::os::unix::fs::FileTypeExt;
    let [r] = redirects else { return None };
    if !(0..=2).contains(&r.fd) || r.word != "/dev/null" {
        return None;
    }
    let flags = match r.target {
        Target::Read => libc::O_RDONLY,
        Target::Write | Target::Clobber => libc::O_WRONLY,
        Target::Append => libc::O_WRONLY | libc::O_APPEND,
        _ => return None,
    };
    let raw = unsafe {
        libc::open(
            c"/dev/null".as_ptr(),
            flags | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOCTTY,
        )
    };
    if raw < 0 {
        return None;
    }
    // SAFETY: open returned a new descriptor and this File takes sole ownership.
    let file = unsafe { std::fs::File::from_raw_fd(raw) };
    if !file.metadata().ok()?.file_type().is_char_device()
        || unsafe { libc::fcntl(raw, libc::F_SETFL, flags) } < 0
    {
        return None;
    }
    Some((r.fd, super::private_fd(file.into()).ok()?))
}

pub(super) fn open_redirect(path: &str, flags: i32) -> Option<OwnedFd> {
    let bytes = path.as_bytes();
    let mut short = [0u8; 256];
    let owned;
    let path = if bytes.len() < short.len() {
        short[..bytes.len()].copy_from_slice(bytes);
        CStr::from_bytes_with_nul(&short[..=bytes.len()]).ok()
    } else {
        owned = CString::new(bytes).ok();
        owned.as_deref()
    };
    let Some(path) = path else {
        diagnostic!("redirection: path contains NUL");
        return None;
    };
    if let Some(device) = path.to_bytes().strip_prefix(b"/dev/")
        && let Some((kind, address)) = device
            .strip_prefix(b"tcp/")
            .map(|a| (libc::SOCK_STREAM, a))
            .or_else(|| device.strip_prefix(b"udp/").map(|a| (libc::SOCK_DGRAM, a)))
    {
        return network_redirect(path, address, kind);
    }
    let mut fd = unsafe { libc::open(path.as_ptr(), flags, 0o666 as libc::c_int) };
    if fd < 0 && flags & libc::O_EXCL != 0 && errno() == libc::EEXIST {
        // Open without truncation and inspect the opened object. A path swap
        // cannot turn a permitted device write into truncation of a regular file.
        fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY) };
        if fd >= 0 {
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            if file.metadata().is_ok_and(|m| !m.is_file()) {
                return Some(file.into());
            }
            write_error(path.to_bytes(), b": cannot overwrite existing file");
            return None;
        }
    }
    if fd < 0 {
        let error = errno();
        let mut buffer = [0 as libc::c_char; 128];
        let reason = if unsafe { libc::strerror_r(error, buffer.as_mut_ptr(), buffer.len()) } == 0 {
            unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes()
        } else {
            b"cannot open redirection"
        };
        write_error_detail(path.to_bytes(), b": ", reason);
        return None;
    }
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Action<'_> {
    pub(super) fn destination(&self) -> RawFd {
        match *self {
            Self::Open(fd, ..)
            | Self::HereInput(fd, _)
            | Self::Dup(_, fd)
            | Self::Close(fd)
            | Self::CloseMoved(fd) => fd,
        }
    }

    fn references(&self, fd: RawFd) -> bool {
        self.destination() == fd
            || match self {
                Self::Dup(src, _) => *src == fd,
                Self::Open(_, path, _) => path_fd(path) == Some(fd),
                _ => false,
            }
    }

    fn references_extra(&self) -> bool {
        self.destination() >= 3
            || match self {
                Self::Dup(src, _) => *src >= 3,
                Self::Open(_, path, _) => path_fd(path).is_some_and(|n| n >= 3),
                _ => false,
            }
    }
}

fn path_fd(path: &str) -> Option<RawFd> {
    let bytes = path.as_bytes();
    let number = bytes
        .strip_prefix(b"/dev/fd/")
        .or_else(|| bytes.strip_prefix(b"/proc/self/fd/"))?;
    std::str::from_utf8(number).ok()?.parse().ok()
}

pub(super) fn protect_fds(actions: &[Action<'_>]) -> io::Result<()> {
    if actions.iter().any(Action::references_extra) {
        fd::protect(|fd| actions.iter().any(|a| a.references(fd)))?;
    }
    Ok(())
}

pub(super) fn apply(actions: &[Action<'_>], pipe_limit: usize) -> bool {
    if let Err(e) = protect_fds(actions) {
        diagnostic!("redirection: {e}");
        return false;
    }
    apply_protected(actions, pipe_limit).is_ok()
}

pub(super) fn apply_protected(actions: &[Action<'_>], pipe_limit: usize) -> Result<(), usize> {
    for (index, a) in actions.iter().enumerate() {
        match a {
            Action::HereInput(dest, body) => {
                let redirected = here_input::open(body, pipe_limit).and_then(|fd| {
                    if fd.as_raw_fd() != *dest {
                        return duplicate(fd.as_raw_fd(), *dest);
                    }
                    if unsafe { libc::fcntl(*dest, libc::F_SETFD, 0) } < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    let _ = fd.into_raw_fd();
                    Ok(())
                });
                if let Err(error) = redirected {
                    diagnostic!("here-document: {error}");
                    return Err(index);
                }
            }
            Action::Open(dest, path, flags) => {
                let Some(fd) = open_redirect(path, *flags) else {
                    return Err(index);
                };
                if fd.as_raw_fd() == *dest {
                    let _ = fd.into_raw_fd();
                } else if duplicate(fd.as_raw_fd(), *dest).is_err() {
                    write_error(path.as_bytes(), b": cannot redirect");
                    return Err(index);
                }
            }
            Action::Dup(src, dst) => {
                if let Err(error) = duplicate(*src, *dst) {
                    if error.raw_os_error() == Some(libc::EBADF) {
                        write_error(b"redirection", b": Bad file descriptor");
                    } else {
                        write_error(b"redirection", b": cannot duplicate file descriptor");
                    }
                    return Err(index);
                }
            }
            Action::Close(fd) | Action::CloseMoved(fd) => {
                unsafe { libc::close(*fd) };
            }
        }
    }
    Ok(())
}

fn capture_file() -> io::Result<fd::PrivateFile> {
    use std::os::fd::BorrowedFd;
    #[cfg(target_os = "linux")]
    {
        // The filesystem supplies an unnamed regular file with a private offset.
        let raw = unsafe {
            libc::open(
                c"/tmp".as_ptr(),
                libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
                0o600 as libc::c_uint,
            )
        };
        if raw >= 0 {
            return fd::PrivateFile::new(unsafe { std::fs::File::from_raw_fd(raw) });
        }
    }
    let stream = unsafe { libc::tmpfile() };
    if stream.is_null() {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { fd::PrivateFile::duplicate(BorrowedFd::borrow_raw(libc::fileno(stream))) };
    unsafe {
        libc::fclose(stream);
    }
    file
}

// The temporary output and the saved stdout are both private descriptors. A
// redirect inside the body can reuse their numbers without breaking restoration.
pub(crate) fn capture_stdout(run: impl FnOnce() -> Flow) -> io::Result<(Flow, Vec<u8>)> {
    use std::io::{Read, Seek, Write};
    io::stdout().flush()?;
    let saved = SavedFd::capture(&Action::Dup(1, 1), &[])?;
    let mut file = capture_file()?;
    duplicate(file.as_raw_fd(), 1)?;
    let flow = run();
    let flushed = io::stdout().flush();
    saved.restore(1)?;
    flushed?;
    file.rewind()?;
    let mut output = Vec::new();
    file.read_to_end(&mut output)?;
    Ok((flow, output))
}

pub(super) fn fd_replaced(session: &mut Session, fd: RawFd) {
    session.stdin_replaced |= fd == 0;
    session.substitutions.replaced(fd);
    coprocess::replaced(session, fd);
}

pub(super) enum SavedFd {
    Unchanged,
    Closed,
    Open(fd::PrivateFile),
}

impl SavedFd {
    pub(super) fn restore_all(saved: impl IntoIterator<Item = (RawFd, Self)>) -> io::Result<()> {
        let mut result = Ok(());
        for (fd, original) in saved {
            if let Err(e) = original.restore(fd) {
                result = Err(e);
            }
        }
        result
    }

    pub(super) fn capture(action: &Action<'_>, actions: &[Action<'_>]) -> io::Result<Self> {
        Ok(Self::reserve(action, actions)?
            .map_or(Self::Closed, |f| Self::Open(fd::PrivateFile::track(f))))
    }

    pub(super) fn reserve(
        action: &Action<'_>,
        actions: &[Action<'_>],
    ) -> io::Result<Option<std::fs::File>> {
        let fd = action.destination();
        match fd::duplicate_except(fd, fd::FIRST_NONSTANDARD, |n| {
            actions.iter().any(|a| a.references(n))
        }) {
            Ok(file) => Ok(Some(file)),
            Err(e) if e.raw_os_error() == Some(libc::EBADF) => Ok(None),
            Err(e) => {
                // GNU Bash closes the target of n>&- even when reserving its
                // undo descriptor fails. A failed duplicate keeps both ends.
                if matches!(action, Action::Close(_)) {
                    unsafe { libc::close(fd) };
                }
                Err(e)
            }
        }
    }

    pub(super) fn restore(self, fd: RawFd) -> io::Result<()> {
        if fd >= fd::FIRST_NONSTANDARD && !matches!(self, Self::Unchanged) {
            fd::protect(|current| current == fd)?;
        }
        match self {
            Self::Unchanged => {}
            Self::Closed => {
                unsafe { libc::close(fd) };
            }
            Self::Open(file) => duplicate(file.as_raw_fd(), fd)?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_relocates_a_tracked_file_before_reusing_its_number() {
        let tracked = fd::PrivateFile::new(std::fs::File::open("/dev/null").unwrap()).unwrap();
        let original = tracked.as_raw_fd();
        SavedFd::Closed.restore(original).unwrap();
        assert_ne!(tracked.as_raw_fd(), original);
        assert!(unsafe { libc::fcntl(tracked.as_raw_fd(), libc::F_GETFD) } >= 0);
    }
}
