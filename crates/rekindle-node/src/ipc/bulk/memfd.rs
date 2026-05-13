//! memfd + SCM_RIGHTS zero-copy path for 100Gbps same-host transfers.
//!
//! At 100Gbps (12.5 GB/s), the 2-copy overhead of normal AF_UNIX sends
//! (user→kernel skb, kernel skb→user) saturates memory bandwidth on DDR4.
//! The memfd path eliminates both copies: the sender writes data into a
//! memfd, seals it, and sends the fd via SCM_RIGHTS. The receiver mmaps
//! the same physical pages — zero copies.
//!
//! # Security
//!
//! Every memfd is sealed with F_SEAL_WRITE + F_SEAL_SHRINK + F_SEAL_GROW +
//! F_SEAL_SEAL before sending via SCM_RIGHTS. This prevents the receiver
//! from modifying the sender's data or causing SIGBUS via truncation.
//! The receiver maps with MAP_PRIVATE (copy-on-write) for additional
//! isolation.
//!
//! # Activation criteria
//!
//! The memfd path is used when:
//! 1. Both sender and receiver are on the same host
//! 2. The payload exceeds 1 MiB (below this, memfd/mmap/seal overhead
//!    exceeds the copy savings)
//! 3. The `bulk-memfd` feature flag is enabled

#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
use std::ffi::CString;

/// Create a sealed memfd containing the given data.
///
/// The memfd is created, sized via ftruncate, written via mmap, and
/// sealed in one call. The returned fd is ready for sending via
/// SCM_RIGHTS.
///
/// Seals applied:
/// - `F_SEAL_WRITE`: prevents any further writes
/// - `F_SEAL_SHRINK`: prevents truncation (SIGBUS prevention)
/// - `F_SEAL_GROW`: prevents growth
/// - `F_SEAL_SEAL`: prevents adding/removing seals
///
/// The caller must close the fd after sending.
#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
pub fn create_sealed_memfd(name: &str, data: &[u8]) -> std::io::Result<std::os::unix::io::RawFd> {
    let c_name = CString::new(name).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid memfd name")
    })?;

    let fd = unsafe {
        libc::memfd_create(
            c_name.as_ptr(),
            libc::MFD_ALLOW_SEALING | libc::MFD_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let len = data.len();

    // Size the memfd.
    if unsafe { libc::ftruncate(fd, len as libc::off_t) } < 0 {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::last_os_error());
    }

    if len > 0 {
        // Map for writing.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            return Err(std::io::Error::last_os_error());
        }

        // Copy data into the memfd.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, len);
            libc::madvise(ptr, len, libc::MADV_SEQUENTIAL);
            libc::munmap(ptr, len);
        }
    }

    // F_SEAL_FUTURE_WRITE (Linux 5.1+) prevents new writable mappings.
    const F_SEAL_FUTURE_WRITE: libc::c_int = 0x0010;

    let full_seals = libc::F_SEAL_WRITE
        | libc::F_SEAL_SHRINK
        | libc::F_SEAL_GROW
        | libc::F_SEAL_SEAL
        | F_SEAL_FUTURE_WRITE;

    let seal_result = unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, full_seals) };
    if seal_result < 0 {
        let base_seals = libc::F_SEAL_WRITE
            | libc::F_SEAL_SHRINK
            | libc::F_SEAL_GROW
            | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, base_seals) } < 0 {
            unsafe { libc::close(fd) };
            return Err(std::io::Error::last_os_error());
        }
    }

    Ok(fd)
}

/// A read-only mapping of a received memfd.
///
/// The fd should have been received via SCM_RIGHTS and should be
/// sealed (F_SEAL_WRITE + F_SEAL_SHRINK + F_SEAL_GROW).
/// Uses MAP_PRIVATE for copy-on-write safety — no lock contention
/// with the sender's page cache.
#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
pub struct MemfdMapping {
    ptr: *const u8,
    len: usize,
}

#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
impl MemfdMapping {
    /// Map a received memfd for reading.
    pub fn map_readonly(fd: std::os::unix::io::RawFd) -> std::io::Result<Self> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut stat) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let len = stat.st_size as usize;

        if len == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "memfd has zero size",
            ));
        }

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            ptr: ptr as *const u8,
            len,
        })
    }

    /// Access the mapped data as a byte slice.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Length of the mapped region in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the mapping is empty (always false — zero-size rejected in map_readonly).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
impl Drop for MemfdMapping {
    fn drop(&mut self) {
        // SAFETY: munmap takes *mut c_void by POSIX but only releases
        // page table entries. Casting *const → *mut is safe for this call.
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.len);
        }
    }
}

// SAFETY: MemfdMapping is Send + Sync because:
// 1. ptr is *const u8. No method exposes &mut or the raw pointer.
// 2. memfd is sealed (F_SEAL_WRITE+SHRINK+GROW+SEAL+FUTURE_WRITE).
// 3. MAP_PRIVATE creates an independent copy-on-write mapping.
// 4. PROT_READ only. Concurrent reads of immutable pages are safe.
// 5. Valid from construction until Drop calls munmap.
#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
unsafe impl Send for MemfdMapping {}
#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
unsafe impl Sync for MemfdMapping {}

#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_memfd() {
        let data = vec![0xABu8; 65536];
        let fd = create_sealed_memfd("test-roundtrip", &data).unwrap();

        let mapping = MemfdMapping::map_readonly(fd).unwrap();
        assert_eq!(mapping.len(), data.len());
        assert_eq!(mapping.as_slice(), &data);

        drop(mapping);
        unsafe { libc::close(fd) };
    }

    #[test]
    fn sealed_memfd_rejects_write_mapping() {
        let data = vec![0xABu8; 4096];
        let fd = create_sealed_memfd("test-sealed", &data).unwrap();

        // Attempt MAP_SHARED + PROT_WRITE should fail (F_SEAL_WRITE).
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
        assert_eq!(ptr, libc::MAP_FAILED);

        unsafe { libc::close(fd) };
    }

    #[test]
    fn small_memfd() {
        let data = b"hello memfd";
        let fd = create_sealed_memfd("test-small", data).unwrap();

        let mapping = MemfdMapping::map_readonly(fd).unwrap();
        assert_eq!(mapping.as_slice(), data);
        assert_eq!(mapping.len(), data.len());
        assert!(!mapping.is_empty());

        drop(mapping);
        unsafe { libc::close(fd) };
    }

    #[test]
    fn large_memfd() {
        let data = vec![0xCDu8; 10 * 1024 * 1024]; // 10 MiB
        let fd = create_sealed_memfd("test-large", &data).unwrap();

        let mapping = MemfdMapping::map_readonly(fd).unwrap();
        assert_eq!(mapping.len(), data.len());
        assert_eq!(&mapping.as_slice()[..1024], &data[..1024]);
        assert_eq!(
            &mapping.as_slice()[data.len() - 1024..],
            &data[data.len() - 1024..]
        );

        drop(mapping);
        unsafe { libc::close(fd) };
    }
}
