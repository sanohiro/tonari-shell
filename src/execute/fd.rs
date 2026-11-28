use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, RawFd};
use std::rc::{Rc, Weak};

pub(crate) const FIRST_NONSTANDARD: RawFd = 3;
pub(crate) const SHELL_FD_MIN: RawFd = 10;
pub(crate) const PROCESS_SUBSTITUTION_MIN: RawFd = 64;

// Rust opens /dev/null on missing standard descriptors before calling main.
// A shell must preserve an inherited close for both builtins and children.
mod standard_at_exec {
    use std::sync::atomic::{AtomicU8, Ordering};

    static CLOSED: AtomicU8 = AtomicU8::new(0);

    #[used]
    #[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
    #[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
    static SAVE: extern "C" fn() = {
        extern "C" fn save() {
            let mut mask = 0;
            for fd in 0..3 {
                if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::EBADF)
                {
                    mask |= 1 << fd;
                }
            }
            CLOSED.store(mask, Ordering::Relaxed);
        }
        save
    };

    pub(crate) fn restore() {
        let mut mask = CLOSED.load(Ordering::Relaxed);
        while mask != 0 {
            unsafe { libc::close(mask.trailing_zeros() as libc::c_int) };
            mask &= mask - 1;
        }
    }
}

pub(crate) use standard_at_exec::restore as restore_standard_at_exec;

// Only the abandoned stack of a forked script interpreter holds an empty slot.
type FileSlot = RefCell<Option<File>>;

thread_local! {
    static PRIVATE: RefCell<Vec<Weak<FileSlot>>> = const { RefCell::new(Vec::new()) };
}

// Scripts, the controlling tty and temporary redirection backups can all outlive
// a shell command. Every owner observes the new number when a user needs that fd.
#[derive(Clone)]
pub(crate) struct PrivateFile(Rc<FileSlot>);

impl PrivateFile {
    pub(crate) fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;
        self.0.borrow().as_ref().unwrap().read_at(buffer, offset)
    }

    pub(crate) fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.0.borrow().as_ref().unwrap().metadata()
    }

    pub(crate) fn duplicate(fd: BorrowedFd<'_>) -> io::Result<Self> {
        let raw = fd.as_raw_fd();
        let copy = duplicate_except(raw, SHELL_FD_MIN, |_| false)
            .or_else(|_| duplicate_except(raw, FIRST_NONSTANDARD, |_| false))?;
        Ok(Self::track(copy))
    }

    pub(crate) fn new(file: File) -> io::Result<Self> {
        let fd = file.as_raw_fd();
        let file = if fd < SHELL_FD_MIN {
            match duplicate_except(fd, SHELL_FD_MIN, |_| false) {
                Ok(copy) => copy,
                Err(_) if fd >= 3 => file,
                Err(_) => duplicate_except(fd, FIRST_NONSTANDARD, |_| false)?,
            }
        } else {
            file
        };
        Ok(Self::track(file))
    }

    pub(super) fn track(file: File) -> Self {
        let file = Rc::new(RefCell::new(Some(file)));
        PRIVATE.with_borrow_mut(|entries| {
            entries.retain(|entry| entry.strong_count() != 0);
            entries.push(Rc::downgrade(&file));
        });
        Self(file)
    }
}

// A forked child entering a fresh interpreter never returns to its old stack.
// Release its registered close-on-exec files before checking for foreign fds.
pub(super) fn close_in_script_child() {
    PRIVATE.with_borrow_mut(|files| {
        for file in files.drain(..).filter_map(|file| file.upgrade()) {
            file.borrow_mut().take();
        }
    });
}

pub(super) fn duplicate_except(
    fd: RawFd,
    mut minimum: RawFd,
    reserved: impl Fn(RawFd) -> bool,
) -> io::Result<File> {
    loop {
        let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, minimum) };
        if copy < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(copy) };
        if !reserved(copy) {
            return Ok(file);
        }
        minimum = copy
            .checked_add(1)
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EMFILE))?;
    }
}

pub(super) fn protect(reserved: impl Fn(RawFd) -> bool) -> io::Result<()> {
    PRIVATE.with_borrow(|files| {
        for file in files.iter().filter_map(Weak::upgrade) {
            let current = file.borrow().as_ref().unwrap().as_raw_fd();
            if reserved(current) {
                let copy = duplicate_except(current, SHELL_FD_MIN, &reserved)
                    .or_else(|_| duplicate_except(current, FIRST_NONSTANDARD, &reserved))?;
                *file.borrow_mut() = Some(copy);
            }
        }
        Ok(())
    })
}

pub(crate) fn is_private(fd: RawFd) -> bool {
    fd >= 3
        && PRIVATE.with_borrow(|entries| {
            entries
                .iter()
                .filter_map(Weak::upgrade)
                .any(|f| f.borrow().as_ref().unwrap().as_raw_fd() == fd)
        })
}

impl AsRawFd for PrivateFile {
    fn as_raw_fd(&self) -> RawFd {
        self.0.borrow().as_ref().unwrap().as_raw_fd()
    }
}
impl Read for PrivateFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().as_mut().unwrap().read(buf)
    }
}
impl Seek for PrivateFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().as_mut().unwrap().seek(pos)
    }
}
