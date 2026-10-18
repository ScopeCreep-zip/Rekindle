//! The shell's hooks (plan E1.1, ADR 0020): where the Host waits on the
//! GUI's not-yet-ported stages during an unlock and its teardown. The Host
//! owns the lifecycle; the GUI implements this with its existing code
//! (`src-tauri/src/services/shell_session.rs`), each domain port deletes
//! its part, and F1 deletes the trait. `rekindled` has no hooks.
//!
//! The shape is the delegate pattern of core-plus-GUI apps (Keybase's
//! service-called UI protocols, matrix-rust-sdk's delegate callbacks): the
//! core calls the frontend at fixed points and waits only where it must —
//! the shell's unported unlock stages run inside the Host's unlock scope
//! and can abort the unlock, and its leave of calls runs before the scope
//! is cancelled (C7.27).

use std::sync::Arc;

use rekindle_crypto::signal::SignalSessionManager;
use rekindle_db::SqliteFriendStore;
use rekindle_lifecycle::SessionScope;
use zeroize::Zeroizing;

/// What an unlock hands the shell: the identity and the keys its unported
/// stages still read.
pub struct UnlockedView {
    pub public_key: String,
    pub display_name: String,
    /// The Ed25519 identity secret.
    pub secret: Zeroizing<[u8; 32]>,
    /// The unlock's one Signal manager.
    pub signal: Arc<SignalSessionManager>,
    /// The receive path's friend authority over the identity's database.
    pub friend_store: Arc<SqliteFriendStore>,
    /// A new identity (create), not a returning one (unlock).
    pub created: bool,
}

/// The identity's own records, once the own-records stage has them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnRecordKeys {
    pub profile: Option<String>,
    pub friend_list: Option<String>,
    pub account: Option<String>,
    pub mailbox: Option<String>,
}

/// Why an unlock ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    /// `Lock`.
    Lock,
    /// `Logout`, or a rotation that moved the identity to a new key.
    Logout,
    /// The unlocked identity was deleted or wiped.
    Deleted,
    /// The host is stopping.
    Exit,
    /// The unlock failed after it began; nothing it started is left.
    Failed,
    /// A task of the unlock panicked (its name).
    TaskPanicked(String),
}

/// The GUI's side of an unlock and its end.
#[async_trait::async_trait]
pub trait ShellHooks: Send + Sync {
    /// The identity is installed and the Host's stages are running: start
    /// the shell's unported stages as children of `scope`. An error aborts
    /// the unlock through the teardown.
    ///
    /// # Errors
    /// A stage could not start.
    async fn unlocked(&self, view: &UnlockedView, scope: &Arc<SessionScope>) -> Result<(), String>;

    /// The own-records stage finished: the keys it holds.
    fn own_records(&self, keys: &OwnRecordKeys);

    /// The teardown is starting, before the unlock scope is cancelled: what
    /// must still send while the session can.
    async fn ending(&self);

    /// The teardown finished; the identity is out of the Host.
    async fn ended(&self, reason: EndReason);
}
