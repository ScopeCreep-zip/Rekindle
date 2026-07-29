//! SideChannel — SOCK_SEQPACKET with SCM_RIGHTS. Linux only.
//!
//! Provides the fd-passing plane for arena setup (arena_fd + states_fd)
//! and future DMA-BUF fd delivery. All fd-carrying messages are atomic
//! (SEQPACKET preserves message boundaries).

use std::io;
use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

#[derive(Debug)]
pub enum SideChannelError {
    CreateFailed(io::Error),
    SendFailed(io::Error),
    RecvFailed(io::Error),
    Closed,
    WouldBlock,
    InvalidTag,
    UnexpectedTag(u8),
    NoFdsReceived,
    WrongCmsgType,
    WrongFdCount { expected: usize, actual: usize },
}

impl std::fmt::Display for SideChannelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateFailed(e) => write!(f, "socketpair failed: {e}"),
            Self::SendFailed(e) => write!(f, "sendmsg failed: {e}"),
            Self::RecvFailed(e) => write!(f, "recvmsg failed: {e}"),
            Self::Closed => write!(f, "peer closed"),
            Self::WouldBlock => write!(f, "would block"),
            Self::InvalidTag => write!(f, "invalid tag in received message"),
            Self::UnexpectedTag(t) => write!(f, "unexpected tag: 0x{t:02x}"),
            Self::NoFdsReceived => write!(f, "no fds in SCM_RIGHTS cmsg"),
            Self::WrongCmsgType => write!(f, "wrong cmsg type"),
            Self::WrongFdCount { expected, actual } => {
                write!(f, "expected {expected} fds, got {actual}")
            }
        }
    }
}

impl std::error::Error for SideChannelError {}

/// Tag accompanying a file descriptor on the SideChannel.
#[derive(Debug, Clone)]
pub struct FdTag {
    pub stream_id: u8,
    pub flags: u8,
    pub payload_size: u64,
    pub payload_id: u32,
    pub fd: RawFd,
}

// RawFd is i32 on all Linux targets.
const FD_BYTE_SIZE: u32 = 4;
const _: () = assert!(core::mem::size_of::<RawFd>() == FD_BYTE_SIZE as usize);

pub struct SideChannel {
    fd: RawFd,
}

impl SideChannel {
    pub fn fd(&self) -> RawFd {
        self.fd
    }

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
            return Err(SideChannelError::CreateFailed(io::Error::last_os_error()));
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

        // SAFETY: CMSG_SPACE computes the correct aligned buffer size for one fd.
        let cmsg_space = unsafe { libc::CMSG_SPACE(FD_BYTE_SIZE) } as usize;
        let mut cmsg_buf = vec![0u8; cmsg_space];

        let mut mhdr: libc::msghdr = unsafe { core::mem::zeroed() };
        mhdr.msg_iov = &raw mut iov;
        mhdr.msg_iovlen = 1;
        mhdr.msg_control = cmsg_buf.as_mut_ptr().cast();
        mhdr.msg_controllen = cmsg_space;

        // SAFETY: CMSG_FIRSTHDR returns a valid pointer into cmsg_buf.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const mhdr);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(FD_BYTE_SIZE) as usize;
            core::ptr::copy_nonoverlapping(
                (&raw const tag.fd).cast::<u8>(),
                libc::CMSG_DATA(cmsg),
                core::mem::size_of::<RawFd>(),
            );
        }

        // SAFETY: mhdr is correctly constructed with valid iov and cmsg.
        let sent = unsafe { libc::sendmsg(self.fd, &raw const mhdr, libc::MSG_NOSIGNAL) };
        if sent < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
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

        let cmsg_space = unsafe { libc::CMSG_SPACE(FD_BYTE_SIZE) } as usize;
        let mut cmsg_buf = vec![0u8; cmsg_space];

        let mut mhdr: libc::msghdr = unsafe { core::mem::zeroed() };
        mhdr.msg_iov = &raw mut iov;
        mhdr.msg_iovlen = 1;
        mhdr.msg_control = cmsg_buf.as_mut_ptr().cast();
        mhdr.msg_controllen = cmsg_space;

        // SAFETY: mhdr is correctly constructed. recvmsg fills iov and cmsg.
        let received = unsafe { libc::recvmsg(self.fd, &raw mut mhdr, 0) };
        if received < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                return Err(SideChannelError::WouldBlock);
            }
            return Err(SideChannelError::RecvFailed(err));
        }
        if received == 0 {
            return Err(SideChannelError::Closed);
        }
        if received.unsigned_abs() < 14 {
            return Err(SideChannelError::InvalidTag);
        }

        let stream_id = iov_buf[0];
        let flags = iov_buf[1];
        let payload_size = u64::from_le_bytes(iov_buf[2..10].try_into().unwrap());
        let payload_id = u32::from_le_bytes(iov_buf[10..14].try_into().unwrap());

        let mut fd: RawFd = -1;
        // SAFETY: CMSG_FIRSTHDR returns a valid pointer or null.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const mhdr);
            if !cmsg.is_null()
                && (*cmsg).cmsg_level == libc::SOL_SOCKET
                && (*cmsg).cmsg_type == libc::SCM_RIGHTS
            {
                core::ptr::copy_nonoverlapping(
                    libc::CMSG_DATA(cmsg),
                    (&raw mut fd).cast::<u8>(),
                    core::mem::size_of::<RawFd>(),
                );
            }
        }

        if fd < 0 {
            return Err(SideChannelError::RecvFailed(
                io::Error::other("no fd in cmsg"),
            ));
        }

        Ok(FdTag { stream_id, flags, payload_size, payload_id, fd })
    }
}

impl AsRawFd for SideChannel {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl Drop for SideChannel {
    fn drop(&mut self) {
        // SAFETY: fd is a valid socket from socketpair.
        unsafe { libc::close(self.fd); }
    }
}

// ── Bootstrap helpers ───────────────────────────────────────────

/// Send a single file descriptor over an existing SOCK_STREAM connection
/// via SCM_RIGHTS. Used to bootstrap the SEQPACKET sidechannel.
pub fn send_fd_over_stream(stream_fd: RawFd, fd_to_send: RawFd) -> Result<(), SideChannelError> {
    let mut iov_buf = [0x01u8];
    let mut iov = libc::iovec {
        iov_base: iov_buf.as_mut_ptr().cast(),
        iov_len: 1,
    };

    let cmsg_space = unsafe { libc::CMSG_SPACE(FD_BYTE_SIZE) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let mut mhdr: libc::msghdr = unsafe { core::mem::zeroed() };
    mhdr.msg_iov = &raw mut iov;
    mhdr.msg_iovlen = 1;
    mhdr.msg_control = cmsg_buf.as_mut_ptr().cast();
    mhdr.msg_controllen = cmsg_space;

    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const mhdr);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(FD_BYTE_SIZE) as usize;
        core::ptr::copy_nonoverlapping(
            (&raw const fd_to_send).cast::<u8>(),
            libc::CMSG_DATA(cmsg),
            core::mem::size_of::<RawFd>(),
        );
    }

    let sent = unsafe { libc::sendmsg(stream_fd, &raw const mhdr, libc::MSG_NOSIGNAL) };
    if sent < 0 {
        return Err(SideChannelError::SendFailed(io::Error::last_os_error()));
    }
    Ok(())
}

/// Receive a single file descriptor from an existing SOCK_STREAM connection.
pub fn recv_fd_from_stream(stream_fd: RawFd) -> Result<RawFd, SideChannelError> {
    let mut iov_buf = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: iov_buf.as_mut_ptr().cast(),
        iov_len: 1,
    };

    let cmsg_space = unsafe { libc::CMSG_SPACE(FD_BYTE_SIZE) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let mut mhdr: libc::msghdr = unsafe { core::mem::zeroed() };
    mhdr.msg_iov = &raw mut iov;
    mhdr.msg_iovlen = 1;
    mhdr.msg_control = cmsg_buf.as_mut_ptr().cast();
    mhdr.msg_controllen = cmsg_space;

    let received = unsafe { libc::recvmsg(stream_fd, &raw mut mhdr, 0) };
    if received < 0 {
        return Err(SideChannelError::RecvFailed(io::Error::last_os_error()));
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
            core::ptr::copy_nonoverlapping(
                libc::CMSG_DATA(cmsg),
                (&raw mut fd).cast::<u8>(),
                core::mem::size_of::<RawFd>(),
            );
        }
    }

    if fd < 0 {
        return Err(SideChannelError::RecvFailed(
            io::Error::other("no fd in SCM_RIGHTS cmsg"),
        ));
    }

    Ok(fd)
}

// ── Arena fd delivery (atomic 2-fd SEQPACKET) ───────────────────

/// Send arena_fd and states_fd in a single atomic SEQPACKET message.
/// Tag byte 0x01 identifies this as arena fd delivery.
///
/// Both fds are delivered atomically — the receiver either gets both
/// or neither. No partial-setup window.
pub fn send_arena_fds(
    sidechannel: &SideChannel,
    arena_fd: BorrowedFd<'_>,
    states_fd: BorrowedFd<'_>,
) -> Result<(), SideChannelError> {
    let mut tag: [u8; 1] = [0x01];
    let fds = [arena_fd.as_raw_fd(), states_fd.as_raw_fd()];

    let mut iov = libc::iovec {
        iov_base: tag.as_mut_ptr().cast(),
        iov_len: 1,
    };

    let fd_payload_size = (2 * core::mem::size_of::<i32>()) as u32;
    let cmsg_space = unsafe { libc::CMSG_SPACE(fd_payload_size) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr().cast();
    msg.msg_controllen = cmsg_space;

    // SAFETY: cmsg_buf is large enough for CMSG_SPACE(8). CMSG_FIRSTHDR
    // returns a valid pointer into our buffer.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(fd_payload_size) as usize;
        core::ptr::copy_nonoverlapping(
            fds.as_ptr(),
            libc::CMSG_DATA(cmsg).cast::<i32>(),
            2,
        );
    }

    let sent = unsafe {
        libc::sendmsg(sidechannel.as_raw_fd(), &raw const msg, libc::MSG_NOSIGNAL)
    };
    if sent < 0 {
        return Err(SideChannelError::SendFailed(io::Error::last_os_error()));
    }

    Ok(())
}

/// Receive arena_fd and states_fd from a single SEQPACKET message.
/// Returns (arena_fd, states_fd) as OwnedFds with CLOEXEC set.
pub fn recv_arena_fds(
    sidechannel: &SideChannel,
) -> Result<(OwnedFd, OwnedFd), SideChannelError> {
    let mut tag = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: tag.as_mut_ptr().cast(),
        iov_len: 1,
    };

    let fd_payload_size = (2 * core::mem::size_of::<i32>()) as u32;
    let cmsg_space = unsafe { libc::CMSG_SPACE(fd_payload_size) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_space];

    let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr().cast();
    msg.msg_controllen = cmsg_space;

    let received = unsafe {
        libc::recvmsg(
            sidechannel.as_raw_fd(),
            &raw mut msg,
            libc::MSG_CMSG_CLOEXEC,
        )
    };
    if received < 0 {
        return Err(SideChannelError::RecvFailed(io::Error::last_os_error()));
    }
    if received != 1 || tag[0] != 0x01 {
        return Err(SideChannelError::UnexpectedTag(tag[0]));
    }

    // SAFETY: CMSG_FIRSTHDR returns a valid pointer or null.
    let cmsg = unsafe { libc::CMSG_FIRSTHDR(&raw const msg) };
    if cmsg.is_null() {
        return Err(SideChannelError::NoFdsReceived);
    }

    let cmsg_ref = unsafe { &*cmsg };
    if cmsg_ref.cmsg_level != libc::SOL_SOCKET
        || cmsg_ref.cmsg_type != libc::SCM_RIGHTS
    {
        return Err(SideChannelError::WrongCmsgType);
    }

    let cmsg_data_len = cmsg_ref.cmsg_len - unsafe { libc::CMSG_LEN(0) as usize };
    let fd_count = cmsg_data_len / core::mem::size_of::<i32>();

    if fd_count != 2 {
        // Close any partially received fds to prevent leaks.
        let fd_ptr: *const i32 = unsafe { libc::CMSG_DATA(cmsg) }.cast();
        for i in 0..fd_count {
            unsafe { libc::close(*fd_ptr.add(i)) };
        }
        return Err(SideChannelError::WrongFdCount {
            expected: 2,
            actual: fd_count,
        });
    }

    let fd_ptr: *const i32 = unsafe { libc::CMSG_DATA(cmsg) }.cast();
    let arena_raw = unsafe { *fd_ptr };
    let states_raw = unsafe { *fd_ptr.add(1) };

    // SAFETY: Both fds are valid (received via SCM_RIGHTS with MSG_CMSG_CLOEXEC).
    Ok((
        unsafe { OwnedFd::from_raw_fd(arena_raw) },
        unsafe { OwnedFd::from_raw_fd(states_raw) },
    ))
}
