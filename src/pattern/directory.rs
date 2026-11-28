use std::ffi::{CStr, CString};
use std::ops::ControlFlow;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub(super) fn visit_names(
    dir: &str,
    directories_only: bool,
    skip_dots: bool,
    prefix: &[u8],
    mut visit: impl FnMut(&CStr, u8),
) {
    let Ok(path) = CString::new(dir) else { return };
    let _ = read_names::<false>(&path, false, prefix, &mut |name, kind| {
        let Some(name) = name else {
            return ControlFlow::Continue(());
        };
        // A symlink or an unknown entry type still needs a pathname lookup.
        if (!directories_only || matches!(kind, libc::DT_DIR | libc::DT_LNK | libc::DT_UNKNOWN))
            && (!skip_dots || !matches!(name.to_bytes(), b"." | b".."))
        {
            visit(name, kind);
        }
        ControlFlow::Continue(())
    });
}

pub(crate) fn scan(
    path: &Path,
    prefix: &[u8],
    visitor: &mut impl FnMut(Option<&CStr>, u8) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return ControlFlow::Continue(());
    };
    read_names::<false>(&path, true, prefix, visitor).unwrap_or(ControlFlow::Continue(()))
}

// Only a complete, successful listing may populate a reusable directory cache.
pub(crate) fn scan_checked(
    path: &Path,
    prefix: &[u8],
    visitor: &mut impl FnMut(Option<&CStr>, u8) -> ControlFlow<()>,
) -> Option<ControlFlow<()>> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    read_names::<true>(&path, true, prefix, visitor)
}

#[cfg(not(target_os = "linux"))]
fn read_names<const CHECK_END: bool>(
    path: &CStr,
    _interruptible: bool,
    prefix: &[u8],
    visit: &mut impl FnMut(Option<&CStr>, u8) -> ControlFlow<()>,
) -> Option<ControlFlow<()>> {
    use std::ptr::NonNull;

    struct DirectoryStream(NonNull<libc::DIR>);
    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0.as_ptr()) };
        }
    }

    let stream = NonNull::new(unsafe { libc::opendir(path.as_ptr()) })?;
    let stream = DirectoryStream(stream);
    let errno = CHECK_END.then(|| unsafe { libc::__error() });
    loop {
        // POSIX distinguishes EOF from an error by errno on a null result.
        let entry = unsafe {
            if let Some(errno) = errno {
                *errno = 0;
            }
            libc::readdir(stream.0.as_ptr())
        };
        if entry.is_null() {
            return (errno.is_none_or(|p| unsafe { *p } == 0)).then_some(ControlFlow::Continue(()));
        }
        // readdir owns the name until the next call on this stream.
        let name = unsafe {
            // Darwin returns the length with the directory entry, excluding its terminator.
            let bytes = std::slice::from_raw_parts(
                std::ptr::addr_of!((*entry).d_name).cast::<u8>(),
                usize::from((*entry).d_namlen) + 1,
            );
            CStr::from_bytes_with_nul_unchecked(bytes)
        };
        if visit(
            name.to_bytes().starts_with(prefix).then_some(name),
            unsafe { (*entry).d_type },
        )
        .is_break()
        {
            return Some(ControlFlow::Break(()));
        }
    }
}

#[cfg(target_os = "linux")]
fn read_names<const CHECK_END: bool>(
    path: &CStr,
    interruptible: bool,
    prefix: &[u8],
    visit: &mut impl FnMut(Option<&CStr>, u8) -> ControlFlow<()>,
) -> Option<ControlFlow<()>> {
    use std::mem::offset_of;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return None;
    }
    let dir = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut bytes = std::mem::MaybeUninit::<[u8; 32768]>::uninit();
    // A queued key can finish completion after a few entries. Fetch the first
    // 2 KiB promptly, then amortize syscalls over 32 KiB for a full traversal.
    const CAPACITY: usize = 32768;
    let mut request = if interruptible { 2048 } else { CAPACITY };
    const RECLEN: usize = offset_of!(libc::dirent64, d_reclen);
    const TYPE: usize = offset_of!(libc::dirent64, d_type);
    const NAME: usize = offset_of!(libc::dirent64, d_name);
    loop {
        let count = unsafe {
            libc::syscall(
                libc::SYS_getdents64,
                dir.as_raw_fd(),
                bytes.as_mut_ptr(),
                request,
            )
        };
        if count <= 0 {
            if count < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            return (count == 0).then_some(ControlFlow::Continue(()));
        }
        request = CAPACITY;
        // getdents64 initialized exactly the returned prefix of the buffer.
        let mut batch =
            unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u8>(), count as usize) };
        while batch.len() > NAME {
            let len = usize::from(u16::from_ne_bytes([batch[RECLEN], batch[RECLEN + 1]]));
            if len <= NAME || len > batch.len() {
                return None;
            }
            let (record, rest) = batch.split_at(len);
            let raw = &record[NAME..];
            let name = if raw.starts_with(prefix) {
                CStr::from_bytes_until_nul(raw).ok()
            } else {
                None
            };
            if visit(name, record[TYPE]).is_break() {
                return Some(ControlFlow::Break(()));
            }
            batch = rest;
        }
    }
}
