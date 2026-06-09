//! Real SideChannel — SOCK_SEQPACKET with SCM_RIGHTS. Linux only.

use std::os::unix::io::RawFd;

#[derive(Debug)]
pub enum SideChannelError {
    CreateFailed(std::io::Error),
    SendFailed(std::io::Error),
    RecvFailed(std::io::Error),
    Closed,
    WouldBlock,
    InvalidTag,
}

#[derive(Debug)]
pub struct FdTag {
    pub stream_id: u8,
    pub flags: u8,
    pub payload_size: u64,
    pub payload_id: u32,
    pub fd: RawFd,
}

// RawFd is i32 on all Linux targets — 4 bytes. This is a compile-time constant.
const FD_BYTE_SIZE: u32 = 4;
const _: () = assert!(std::mem::size_of::<RawFd>() == FD_BYTE_SIZE as usize);

pub struct SideChannel {
    fd: RawFd,
}

impl SideChannel {
    pub fn fd(&self) -> RawFd { self.fd }

    /// Wrap an existing raw fd as a SideChannel. The fd must be a valid
    /// SOCK_SEQPACKET socket received via SCM_RIGHTS.
    pub fn from_raw_fd(fd: RawFd) -> Self {
        assert!(fd >= 0, "SideChannel::from_raw_fd called with invalid fd {fd}");
        Self { fd }
    }

    pub fn create_pair() -> Result<(SideChannel, SideChannel), SideChannelError> {
        let mut fds: [RawFd; 2] = [-1; 2];
        // SAFETY: socketpair is a safe Linux syscall. fds is a valid 2-element array.
        let rc = unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                0,
                fds.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return Err(SideChannelError::CreateFailed(std::io::Error::last_os_error()));
        }
        Ok((SideChannel { fd: fds[0] }, SideChannel { fd: fds[1] }))
    }

    pub fn send_tagged_fd(&self, tag: &FdTag) -> Result<(), SideChannelError> {
        let mut iov_buf = [0u8; 14];
        iov_buf[0] = tag.stream_id;
        iov_buf[1] = tag.flags;
        iov_buf[2..10].copy_from_slice(&tag.payload_size.to_le_bytes());
        iov_buf[10..14].copy_from_slice(&tag.payload_id.to_le_bytes());

        let mut iov = libc::iovec {
            iov_base: iov_buf.as_mut_ptr().cast(),
            iov_len: 14,
        };

        let cmsg_space = unsafe { libc::CMSG_SPACE(FD_BYTE_SIZE) } as usize;
        let mut cmsg_buf = vec![0u8; cmsg_space];

        let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
        mhdr.msg_iov = &raw mut iov;
        mhdr.msg_iovlen = 1;
        mhdr.msg_control = cmsg_buf.as_mut_ptr().cast();
        mhdr.msg_controllen = cmsg_space;

        // SAFETY: CMSG_FIRSTHDR returns a valid pointer into our cmsg_buf
        // because msg_control and msg_controllen are correctly set.
        // We write the fd into the cmsg data area.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const mhdr);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(FD_BYTE_SIZE) as usize;
            std::ptr::copy_nonoverlapping(
                (&raw const tag.fd).cast::<u8>(),
                libc::CMSG_DATA(cmsg),
                std::mem::size_of::<RawFd>(),
            );
        }

        // SAFETY: mhdr is correctly constructed with valid iov and cmsg.
        // MSG_NOSIGNAL prevents SIGPIPE if the peer closed.
        let sent = unsafe { libc::sendmsg(self.fd, &raw const mhdr, libc::MSG_NOSIGNAL) };
        if sent < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return Err(SideChannelError::WouldBlock);
            }
            return Err(SideChannelError::SendFailed(err));
        }
        Ok(())
    }

    pub fn recv_tagged_fd(&self) -> Result<FdTag, SideChannelError> {
        let mut iov_buf = [0u8; 14];
        let mut iov = libc::iovec {
            iov_base: iov_buf.as_mut_ptr().cast(),
            iov_len: 14,
        };

        // SAFETY: CMSG_SPACE computes the aligned buffer size for one fd.
        let cmsg_space = unsafe { libc::CMSG_SPACE(FD_BYTE_SIZE) } as usize;
        let mut cmsg_buf = vec![0u8; cmsg_space];

        // SAFETY: zeroed msghdr is valid.
        let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
        mhdr.msg_iov = &raw mut iov;
        mhdr.msg_iovlen = 1;
        mhdr.msg_control = cmsg_buf.as_mut_ptr().cast();
        mhdr.msg_controllen = cmsg_space;

        // SAFETY: mhdr is correctly constructed. recvmsg fills iov and cmsg.
        let received = unsafe { libc::recvmsg(self.fd, &raw mut mhdr, 0) };
        if received < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return Err(SideChannelError::WouldBlock);
            }
            return Err(SideChannelError::RecvFailed(err));
        }
        if received == 0 {
            return Err(SideChannelError::Closed);
        }
        // received > 0 from the checks above, cast to unsigned is safe.
        if received.unsigned_abs() < 14 {
            return Err(SideChannelError::InvalidTag);
        }

        let stream_id = iov_buf[0];
        let flags = iov_buf[1];
        let payload_size = u64::from_le_bytes(iov_buf[2..10].try_into().unwrap());
        let payload_id = u32::from_le_bytes(iov_buf[10..14].try_into().unwrap());

        // Extract fd from cmsg
        let mut fd: RawFd = -1;
        // SAFETY: CMSG_FIRSTHDR returns a valid pointer into cmsg_buf or null.
        // We check for null before dereferencing. CMSG_DATA returns the fd bytes.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const mhdr);
            if !cmsg.is_null()
                && (*cmsg).cmsg_level == libc::SOL_SOCKET
                && (*cmsg).cmsg_type == libc::SCM_RIGHTS
            {
                std::ptr::copy_nonoverlapping(
                    libc::CMSG_DATA(cmsg),
                    (&raw mut fd).cast::<u8>(),
                    std::mem::size_of::<RawFd>(),
                );
            }
        }

        if fd < 0 {
            return Err(SideChannelError::RecvFailed(
                std::io::Error::other("no fd in cmsg"),
            ));
        }

        Ok(FdTag { stream_id, flags, payload_size, payload_id, fd })
    }
}

impl Drop for SideChannel {
    fn drop(&mut self) {
        // SAFETY: fd is a valid socket from socketpair.
        unsafe { libc::close(self.fd); }
    }
}

// ── Bootstrap helpers: send/recv a single fd over an existing stream ──

/// Send a single file descriptor over an existing SOCK_STREAM connection
/// via SCM_RIGHTS. Used to bootstrap the SEQPACKET sidechannel: the server
/// creates a socketpair and sends one end to the client over the main socket.
///
/// `stream_fd`: raw fd of the SOCK_STREAM connection (main socket).
/// `fd_to_send`: raw fd to transfer to the peer (one end of the sidechannel).
pub fn send_fd_over_stream(stream_fd: RawFd, fd_to_send: RawFd) -> Result<(), SideChannelError> {
    let mut iov_buf = [0x01u8]; // 1-byte sentinel — SCM_RIGHTS requires at least 1 byte of data
    let mut iov = libc::iovec {
        iov_base: iov_buf.as_mut_ptr().cast(),
        iov_len: 1,
    };

    let cmsg_space = unsafe { libc::CMSG_SPACE(FD_BYTE_SIZE) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
    mhdr.msg_iov = &raw mut iov;
    mhdr.msg_iovlen = 1;
    mhdr.msg_control = cmsg_buf.as_mut_ptr().cast();
    mhdr.msg_controllen = cmsg_space;

    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const mhdr);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(FD_BYTE_SIZE) as usize;
        std::ptr::copy_nonoverlapping(
            (&raw const fd_to_send).cast::<u8>(),
            libc::CMSG_DATA(cmsg),
            std::mem::size_of::<RawFd>(),
        );
    }

    let sent = unsafe { libc::sendmsg(stream_fd, &raw const mhdr, libc::MSG_NOSIGNAL) };
    if sent < 0 {
        return Err(SideChannelError::SendFailed(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Receive a single file descriptor from an existing SOCK_STREAM connection
/// via SCM_RIGHTS. The peer must have called `send_fd_over_stream` first.
///
/// `stream_fd`: raw fd of the SOCK_STREAM connection (main socket).
/// Returns the received fd (one end of the sidechannel).
pub fn recv_fd_from_stream(stream_fd: RawFd) -> Result<RawFd, SideChannelError> {
    let mut iov_buf = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: iov_buf.as_mut_ptr().cast(),
        iov_len: 1,
    };

    let cmsg_space = unsafe { libc::CMSG_SPACE(FD_BYTE_SIZE) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
    mhdr.msg_iov = &raw mut iov;
    mhdr.msg_iovlen = 1;
    mhdr.msg_control = cmsg_buf.as_mut_ptr().cast();
    mhdr.msg_controllen = cmsg_space;

    let received = unsafe { libc::recvmsg(stream_fd, &raw mut mhdr, 0) };
    if received < 0 {
        return Err(SideChannelError::RecvFailed(std::io::Error::last_os_error()));
    }
    if received == 0 {
        return Err(SideChannelError::Closed);
    }

    let mut fd: RawFd = -1;
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const mhdr);
        if !cmsg.is_null()
            && (*cmsg).cmsg_level == libc::SOL_SOCKET
            && (*cmsg).cmsg_type == libc::SCM_RIGHTS
        {
            std::ptr::copy_nonoverlapping(
                libc::CMSG_DATA(cmsg),
                (&raw mut fd).cast::<u8>(),
                std::mem::size_of::<RawFd>(),
            );
        }
    }

    if fd < 0 {
        return Err(SideChannelError::RecvFailed(
            std::io::Error::other("no fd in SCM_RIGHTS cmsg"),
        ));
    }

    Ok(fd)
}
