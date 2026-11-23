use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, RawFd};
use std::rc::{Rc, Weak};

thread_local! {
    static PRIVATE: RefCell<Vec<Weak<RefCell<File>>>> = const { RefCell::new(Vec::new()) };
}

// Scripts, the controlling tty and temporary redirection backups can all outlive
// a shell command. Every owner observes the new number when a user needs that fd.
#[derive(Clone)]
pub(crate) struct PrivateFile(Rc<RefCell<File>>);

impl PrivateFile {
    pub(crate) fn duplicate(fd: BorrowedFd<'_>) -> io::Result<Self> {
        let raw = fd.as_raw_fd();
        let copy = duplicate_except(raw, 10, |_| false)
            .or_else(|_| duplicate_except(raw, 3, |_| false))?;
        Ok(Self::track(copy))
    }

    pub(crate) fn new(file: File) -> io::Result<Self> {
        let fd = file.as_raw_fd();
        let file = if fd < 10 {
            match duplicate_except(fd, 10, |_| false) {
                Ok(copy) => copy,
                Err(_) if fd >= 3 => file,
                Err(_) => duplicate_except(fd, 3, |_| false)?,
            }
        } else {
            file
        };
        Ok(Self::track(file))
    }

    pub(super) fn track(file: File) -> Self {
        let file = Rc::new(RefCell::new(file));
        PRIVATE.with_borrow_mut(|entries| {
            entries.retain(|entry| entry.strong_count() != 0);
            entries.push(Rc::downgrade(&file));
        });
        Self(file)
    }
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
            let current = file.borrow().as_raw_fd();
            if reserved(current) {
                let copy = duplicate_except(current, 3, &reserved)?;
                *file.borrow_mut() = copy;
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
                .any(|f| f.borrow().as_raw_fd() == fd)
        })
}

impl AsRawFd for PrivateFile {
    fn as_raw_fd(&self) -> RawFd {
        self.0.borrow().as_raw_fd()
    }
}
impl Read for PrivateFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().read(buf)
    }
}
impl Seek for PrivateFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().seek(pos)
    }
}
