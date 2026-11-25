use std::ffi::CString;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;

// Bash can deliver medium documents through a pipe too. Actual capacity may
// be reduced by the kernel; never wait for a reader that has not started yet.
pub(super) const PIPE_LIMIT: usize = 64 * 1024;

pub(super) fn open(data: &[u8], pipe_limit: usize) -> io::Result<OwnedFd> {
    if data.is_empty() {
        return File::open("/dev/null").map(Into::into);
    }
    // A write within PIPE_BUF fits in a fresh pipe without a concurrent
    // reader, including on macOS. Larger documents use a seekable file.
    if data.len() <= libc::PIPE_BUF {
        let (reader, mut writer) = io::pipe()?;
        writer.write_all(data)?;
        return Ok(reader.into());
    }
    if data.len() <= pipe_limit
        && let Ok((reader, mut writer)) = io::pipe()
        && unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } == 0
    {
        loop {
            match writer.write(data) {
                Ok(n) if n == data.len() => return Ok(reader.into()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // A partial pipe stays private and is dropped in its entirety.
                _ => break,
            }
        }
    }
    temporary(data)
}

fn temporary(data: &[u8]) -> io::Result<OwnedFd> {
    let template = std::env::temp_dir().join("tonari-here-XXXXXX");
    let mut path = CString::new(template.as_os_str().as_bytes())?.into_bytes_with_nul();
    let fd = unsafe { libc::mkstemp(path.as_mut_ptr().cast()) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut writer = unsafe { File::from_raw_fd(fd) };
    // mkstemp honors the caller's umask. The shell still needs to reopen the
    // file for reading when even owner-read permission was masked off.
    if unsafe { libc::fchmod(fd, 0o600) } < 0 {
        let error = io::Error::last_os_error();
        unsafe { libc::unlink(path.as_ptr().cast()) };
        return Err(error);
    }
    // An independently opened read end cannot write to the document or share
    // the writer's offset. Remove the name before storing the document's bytes.
    let fd = unsafe {
        libc::open(
            path.as_ptr().cast(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    let reader = if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    };
    if unsafe { libc::unlink(path.as_ptr().cast()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let reader = reader?;
    let created = writer.metadata()?;
    let reopened = reader.metadata()?;
    if (created.dev(), created.ino()) != (reopened.dev(), reopened.ino()) {
        return Err(io::Error::other("here-document file was replaced"));
    }
    writer.write_all(data)?;
    Ok(reader.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn short_and_large_documents_are_read_only_and_preserve_all_bytes() {
        for len in [
            0,
            1,
            libc::PIPE_BUF,
            8193,
            PIPE_LIMIT,
            PIPE_LIMIT + 1,
            PIPE_LIMIT * 2,
        ] {
            let data = (0..len).map(|i| i as u8).collect::<Vec<_>>();
            let mut file = File::from(open(&data, PIPE_LIMIT).unwrap());
            assert!(file.write(b"!").is_err());
            let mut copy = Vec::new();
            file.read_to_end(&mut copy).unwrap();
            assert_eq!(copy, data);
        }
    }
}
