use std::ffi::{CStr, CString};

pub(super) fn visit_names(
    dir: &str,
    directories_only: bool,
    skip_dots: bool,
    mut visit: impl FnMut(&CStr, u8),
) {
    let Ok(path) = CString::new(dir) else { return };
    read_names(&path, |name, kind| {
        // A symlink or an unknown entry type still needs a pathname lookup.
        if (!directories_only || matches!(kind, libc::DT_DIR | libc::DT_LNK | libc::DT_UNKNOWN))
            && (!skip_dots || !matches!(name.to_bytes(), b"." | b".."))
        {
            visit(name, kind);
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn read_names(path: &CStr, mut visit: impl FnMut(&CStr, u8)) {
    use std::ptr::NonNull;

    struct DirectoryStream(NonNull<libc::DIR>);
    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0.as_ptr()) };
        }
    }

    let Some(stream) = NonNull::new(unsafe { libc::opendir(path.as_ptr()) }) else {
        return;
    };
    let stream = DirectoryStream(stream);
    loop {
        let entry = unsafe { libc::readdir(stream.0.as_ptr()) };
        if entry.is_null() {
            break;
        }
        // readdir owns the name until the next call on this stream.
        let name = unsafe { CStr::from_ptr(std::ptr::addr_of!((*entry).d_name).cast()) };
        visit(name, unsafe { (*entry).d_type });
    }
}

#[cfg(target_os = "linux")]
fn read_names(path: &CStr, mut visit: impl FnMut(&CStr, u8)) {
    use std::mem::offset_of;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return;
    }
    let dir = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut bytes = [0u8; 32768];
    const RECLEN: usize = offset_of!(libc::dirent64, d_reclen);
    const TYPE: usize = offset_of!(libc::dirent64, d_type);
    const NAME: usize = offset_of!(libc::dirent64, d_name);
    loop {
        let count = unsafe {
            libc::syscall(
                libc::SYS_getdents64,
                dir.as_raw_fd(),
                bytes.as_mut_ptr(),
                bytes.len(),
            )
        };
        if count <= 0 {
            if count < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            return;
        }
        let mut batch = &bytes[..count as usize];
        while batch.len() > NAME {
            let len = usize::from(u16::from_ne_bytes([batch[RECLEN], batch[RECLEN + 1]]));
            if len <= NAME || len > batch.len() {
                return;
            }
            let (record, rest) = batch.split_at(len);
            let Ok(name) = CStr::from_bytes_until_nul(&record[NAME..]) else {
                return;
            };
            visit(name, record[TYPE]);
            batch = rest;
        }
    }
}
