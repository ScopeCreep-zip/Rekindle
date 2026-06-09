//! Real memfd operations — Linux only.

use std::os::unix::io::RawFd;

#[derive(Debug)]
pub enum MemfdError {
    CreateFailed(std::io::Error),
    TruncateFailed(std::io::Error),
    MmapFailed(std::io::Error),
    SealFailed(std::io::Error),
    VerifyFailed { expected: [u8; 32], computed: [u8; 32] },
    SizeMismatch { expected: u64, actual: u64 },
}

pub struct SealedMemfd {
    fd: RawFd,
    size: u64,
}

impl SealedMemfd {
    pub fn fd(&self) -> RawFd { self.fd }
    pub fn size(&self) -> u64 { self.size }
}

impl Drop for SealedMemfd {
    fn drop(&mut self) {
        // SAFETY: fd is a valid file descriptor created by memfd_create.
        // Closing it releases the kernel's reference to the memfd.
        unsafe { libc::close(self.fd); }
    }
}

pub struct MemfdMapping {
    ptr: *mut u8,
    len: usize,
}

impl MemfdMapping {
    pub fn as_slice(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: ptr is a valid mmap'd region of len bytes, mapped PROT_READ.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for MemfdMapping {
    fn drop(&mut self) {
        if self.len > 0 {
            // SAFETY: ptr and len are from a successful mmap call.
            unsafe { libc::munmap(self.ptr.cast(), self.len); }
        }
    }
}

pub fn create_memfd(name: &str, size: u64) -> Result<RawFd, MemfdError> {
    let c_name = std::ffi::CString::new(name).expect("name contains null");
    // SAFETY: memfd_create is a safe Linux syscall with a valid C string.
    // The return value is checked for < 0 before use.
    let raw = unsafe {
        libc::syscall(
            libc::SYS_memfd_create,
            c_name.as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    let fd = i32::try_from(raw).unwrap_or(-1);
    if fd < 0 {
        return Err(MemfdError::CreateFailed(std::io::Error::last_os_error()));
    }
    if size > 0 {
        // SAFETY: fd is a valid memfd, ftruncate sets its size.
        // off_t is i64 on 64-bit Linux; size is bounded by available memory.
        let rc = unsafe { libc::ftruncate(fd, i64::try_from(size).expect("size exceeds i64")) };
        if rc != 0 {
            // SAFETY: fd is valid, close releases it after ftruncate failure.
            unsafe { libc::close(fd); }
            return Err(MemfdError::TruncateFailed(std::io::Error::last_os_error()));
        }
    }
    Ok(fd)
}

pub fn write_and_seal(name: &str, data: &[u8]) -> Result<SealedMemfd, MemfdError> {
    let size = data.len() as u64;
    let fd = create_memfd(name, size)?;

    if !data.is_empty() {
        // SAFETY: fd is a valid memfd of `size` bytes. MAP_SHARED so writes persist.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                data.len(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            // SAFETY: fd is valid, close releases it after mmap failure.
            unsafe { libc::close(fd); }
            return Err(MemfdError::MmapFailed(std::io::Error::last_os_error()));
        }
        // SAFETY: ptr is a valid mmap'd region, data.len() <= mapped size.
        // copy_nonoverlapping is valid because src (data) and dst (mmap) don't overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast(), data.len());
            libc::munmap(ptr, data.len());
        }
    }

    // Seal against modification
    // SAFETY: fd is a valid memfd created with MFD_ALLOW_SEALING.
    let rc = unsafe {
        libc::fcntl(
            fd,
            libc::F_ADD_SEALS,
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK,
        )
    };
    if rc != 0 {
        // SAFETY: fd is valid, close releases it after seal failure.
        unsafe { libc::close(fd); }
        return Err(MemfdError::SealFailed(std::io::Error::last_os_error()));
    }

    Ok(SealedMemfd { fd, size })
}

pub fn map_readonly(fd: RawFd, size: u64) -> Result<MemfdMapping, MemfdError> {
    if size == 0 {
        return Ok(MemfdMapping { ptr: std::ptr::NonNull::<u8>::dangling().as_ptr(), len: 0 });
    }
    let map_len = usize::try_from(size).expect("memfd size exceeds usize");
    // SAFETY: fd is a valid memfd, mapping PROT_READ only. map_len > 0.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            map_len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(MemfdError::MmapFailed(std::io::Error::last_os_error()));
    }
    Ok(MemfdMapping { ptr: ptr.cast(), len: map_len })
}

/// Get the size of a memfd via fstat. Returns None on error.
pub fn memfd_size(fd: RawFd) -> Option<u64> {
    unsafe {
        let mut stat: libc::stat = std::mem::zeroed();
        if libc::fstat(fd, &mut stat) != 0 { return None; }
        Some(stat.st_size as u64)
    }
}

/// Close a raw file descriptor.
pub fn close_fd(fd: RawFd) {
    unsafe { libc::close(fd); }
}

pub fn verify_content(mapping: &MemfdMapping, expected_hash: &[u8; 32]) -> Result<(), MemfdError> {
    let computed = *blake3::hash(mapping.as_slice()).as_bytes();
    if &computed != expected_hash {
        return Err(MemfdError::VerifyFailed { expected: *expected_hash, computed });
    }
    Ok(())
}
