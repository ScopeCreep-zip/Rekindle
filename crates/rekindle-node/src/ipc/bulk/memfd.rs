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

    // SAFETY: memfd_create with valid CString name. MFD_CLOEXEC ensures
    // the fd is not leaked to child processes.
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

    #[allow(clippy::cast_possible_wrap)] // len fits in off_t on 64-bit
    // SAFETY: ftruncate on a valid memfd fd sets the size.
    if unsafe { libc::ftruncate(fd, len as libc::off_t) } < 0 {
        // SAFETY: fd is valid, close releases it.
        unsafe { libc::close(fd) };
        return Err(std::io::Error::last_os_error());
    }

    if len > 0 {
        // SAFETY: mmap with MAP_SHARED on the memfd. fd is valid, len > 0.
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
            // SAFETY: fd is valid.
            unsafe { libc::close(fd) };
            return Err(std::io::Error::last_os_error());
        }

        // SAFETY: ptr is a valid MAP_SHARED mapping of len bytes.
        // data.as_ptr() is valid for len bytes. No overlap (different allocations).
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast::<u8>(), len);
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

    // SAFETY: fcntl F_ADD_SEALS on a valid memfd fd.
    let seal_result = unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, full_seals) };
    if seal_result < 0 {
        let base_seals = libc::F_SEAL_WRITE
            | libc::F_SEAL_SHRINK
            | libc::F_SEAL_GROW
            | libc::F_SEAL_SEAL;
        // SAFETY: fallback seals without F_SEAL_FUTURE_WRITE for older kernels.
        if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, base_seals) } < 0 {
            // SAFETY: fd is valid.
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
        // SAFETY: zeroed stat struct is valid. fstat on a valid fd fills it.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: fd is valid, &raw mut stat points to stack-allocated struct.
        if unsafe { libc::fstat(fd, &raw mut stat) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let len = stat.st_size as usize;

        if len == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "memfd has zero size",
            ));
        }

        // SAFETY: mmap with MAP_PRIVATE + PROT_READ on valid fd, len > 0.
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
            ptr: ptr.cast::<u8>().cast_const(),
            len,
        })
    }

    /// Access the mapped data as a byte slice.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: ptr is valid for len bytes (from mmap), immutable
        // (PROT_READ + sealed), and valid until Drop calls munmap.
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
        // ptr and len are from a successful mmap in map_readonly.
        unsafe {
            libc::munmap(self.ptr.cast_mut().cast::<libc::c_void>(), self.len);
        }
    }
}

#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
// SAFETY: MemfdMapping is Send because ptr is derived from mmap (process-wide
// address), the mapping is sealed (immutable), and PROT_READ only. No &mut
// is ever exposed. The mapping is valid from construction until Drop.
unsafe impl Send for MemfdMapping {}

#[cfg(all(target_os = "linux", feature = "bulk-memfd"))]
// SAFETY: MemfdMapping is Sync because all access is through as_slice()
// which returns &[u8] (shared reference). The backing pages are immutable
// (F_SEAL_WRITE + MAP_PRIVATE + PROT_READ). Concurrent reads are safe.
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
        // SAFETY: fd is valid, close releases it.
        unsafe { libc::close(fd) };
    }

    #[test]
    fn sealed_memfd_rejects_write_mapping() {
        let data = vec![0xABu8; 4096];
        let fd = create_sealed_memfd("test-sealed", &data).unwrap();

        // Attempt MAP_SHARED + PROT_WRITE should fail (F_SEAL_WRITE).
        // SAFETY: mmap with invalid seal combination — expected to fail.
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

        // SAFETY: fd is valid.
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
        // SAFETY: fd is valid.
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
        // SAFETY: fd is valid.
        unsafe { libc::close(fd) };
    }
}
