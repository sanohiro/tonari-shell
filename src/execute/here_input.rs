use std::ffi::CString;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;

pub(super) fn open(data: &[u8]) -> io::Result<OwnedFd> {
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
        for len in [0, 1, libc::PIPE_BUF, libc::PIPE_BUF * 64] {
            let data = (0..len).map(|i| i as u8).collect::<Vec<_>>();
            let mut file = File::from(open(&data).unwrap());
            assert!(file.write(b"!").is_err());
            let mut copy = Vec::new();
            file.read_to_end(&mut copy).unwrap();
            assert_eq!(copy, data);
        }
    }
}
