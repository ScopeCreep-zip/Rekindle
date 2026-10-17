//! `HostApi`: the one entry every frontend reaches the host through (plan
//! D3.1). The GUI calls it in-process as [`ConnId::Shell`]; the TUI and CLI
//! reach it over the bus, whose subscriber calls it for each request. F1
//! swaps the GUI's implementation for `DaemonClient`.

pub use rekindle_events::EventStream;
use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
pub use rekindle_types::subscription_events::Cursor;

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
    /// Clearance, then dispatch by `Lane` (`validate_request` runs in
    /// dispatch; D3.3 puts the intent gate between them).
    async fn call(&self, caller: Caller, req: IpcRequest) -> IpcResponse;

    /// The host's event stream from `from` (plan D21): sequenced and
    /// resumable — what followed `from`, then `Resumed`, or
    /// `InvalidSession`; `None` for live frames only.
    fn subscribe(&self, from: Option<Cursor>) -> EventStream;
}

#[async_trait::async_trait]
impl HostApi for super::Host {
    async fn call(&self, caller: Caller, req: IpcRequest) -> IpcResponse {
        self.ctx.call(req, &caller).await
    }

    fn subscribe(&self, from: Option<Cursor>) -> EventStream {
        self.ctx.journal.subscribe(from)
    }
}
