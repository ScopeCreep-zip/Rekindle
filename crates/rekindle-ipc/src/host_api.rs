//! `HostApi`: the one entry every frontend reaches the host through (plans
//! D3.1, E1.1). It lives here, beside `IpcRequest`, so a frontend names it
//! without linking the node: the GUI calls the in-process `Host` through it
//! as [`ConnId::Shell`] today, the TUI and CLI reach it over the bus, and
//! F1's `DaemonClient` implements it for the GUI.

use rekindle_events::EventStream;
use rekindle_types::subscription_events::Cursor;

use crate::error::IpcError;
use crate::message::SecurityLevel;
use crate::protocol::{IpcRequest, IpcResponse};

/// The connection a request came on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnId {
    /// The GUI's in-process call.
    Shell,
    /// A bus connection, by the id the bus server stamped on the frame.
    Bus(u64),
}

/// Who sent a request. Every field is stamped by the host or the bus server
/// from the sender's connection; nothing here comes from the client's own
/// claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub conn: ConnId,
    /// The clearance the registry gives the sender's key at this request
    /// (plan D1); the shell calls at `Admin`.
    pub clearance: SecurityLevel,
    /// The agent name the sender's Noise key is registered under, if any.
    pub verified_name: Option<String>,
    /// The sender's Noise static key, proven by the handshake.
    pub static_key: Option<[u8; 32]>,
}

impl Caller {
    /// The GUI, calling in-process: the user at the window holds the
    /// session it unlocked.
    #[must_use]
    pub fn shell() -> Self {
        Self {
            conn: ConnId::Shell,
            clearance: SecurityLevel::Admin,
            verified_name: None,
            static_key: None,
        }
    }
}

/// The host's request surface.
#[async_trait::async_trait]
pub trait HostApi: Send + Sync {
    /// Clearance, `validate_request`, the intent gate, then dispatch by
    /// lane (plan D3.3).
    async fn call(&self, caller: Caller, req: IpcRequest) -> IpcResponse;

    /// The host's event stream from `from` (plan D21): sequenced and
    /// resumable — what followed `from`, then `Resumed`, or
    /// `InvalidSession`; `None` for live frames only.
    ///
    /// # Errors
    /// The stream cannot be opened (a bus client's connection is gone).
    async fn subscribe(&self, from: Option<Cursor>) -> Result<EventStream, IpcError>;
}
