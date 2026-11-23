use std::fs::File;
use std::io::{self, Read};

pub const WHOLE: usize = 64 * 1024;
pub const MARKS: [[u8; 2]; 2] = [[0xFF, 0xFE], [0xFE, 0xFF]];

pub enum Loaded {
    Text(usize),
    Binary,
    Unmatched,
    Large(File),
}

pub fn fill(mut file: File, buffer: &mut Vec<u8>, mut patient: bool) -> io::Result<Loaded> {
    if buffer.is_empty() {
        buffer.resize(WHOLE, 0);
    }
    let mut length = 0;
    let mut wide = false;
    while length < buffer.len() {
        let rest = &mut buffer[length..];
        let read = if patient {
            file.read(rest)
        } else {
            glance(&file, rest)
        };
        let read = match read {
            Ok(0) => return Ok(Loaded::Text(length)),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if patient || error.kind() == io::ErrorKind::WouldBlock => {
                return Err(error);
            }
            Err(_) => {
                patient = true;
                continue;
            }
        };
        if length == 0 {
            wide = MARKS.iter().any(|mark| buffer[..read].starts_with(mark));
        }
        if !wide && memchr::memchr(0, &buffer[length..length + read]).is_some() {
            return Ok(Loaded::Binary);
        }
        length += read;
    }
    Ok(Loaded::Large(file))
}

#[cfg(all(target_os = "linux", any(target_env = "gnu", target_env = "musl")))]
fn glance(file: &File, buffer: &mut [u8]) -> io::Result<usize> {
    use std::os::fd::AsRawFd;

    const EARLY: libc::off_t = 1 << 20;
    let part = libc::iovec {
        iov_base: buffer.as_mut_ptr().cast(),
        iov_len: buffer.len(),
    };
    // RWF_NOWAIT returns EAGAIN when the data is not in the page cache yet
    let read = unsafe { libc::preadv2(file.as_raw_fd(), &part, 1, -1, libc::RWF_NOWAIT) };
    if read >= 0 {
        return Ok(read as usize);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        unsafe {
            libc::posix_fadvise(file.as_raw_fd(), 0, EARLY, libc::POSIX_FADV_WILLNEED);
        }
    }
    Err(error)
}

#[cfg(not(all(target_os = "linux", any(target_env = "gnu", target_env = "musl"))))]
fn glance(_: &File, _: &mut [u8]) -> io::Result<usize> {
    Err(io::ErrorKind::Unsupported.into())
}
