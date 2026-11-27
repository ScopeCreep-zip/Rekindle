//! A shell-mode netsim node (plan E3.4 S5, Q3): its Host adopts a transport
//! built before it (`TransportSource::Adopted`, `HostMode::Shell`), as the
//! desktop's does (`src-tauri/src/setup.rs`), so the Host paths that branch
//! on the mode (member routes, the mesh, beats, the unlock's shell arm) run
//! as on the desktop.
//!
//! Its inbound handler calls only the Host entries the desktop's Veilid
//! dispatch calls (`src-tauri/src/services/veilid/`: `receive_direct`,
//! `receive_direct_call`, `receive_mek_transfer`, `ingest_head_advanced`,
//! the governance and friends `on_watched_change`, `on_membership_control`,
//! `network_changed`). What that dispatch does in `src-tauri` itself (the
//! unported 1:1 kinds, bootstrap and media-key answers, gossip ingest, the
//! `RequestMEK` responder over the shell's key adapter) has no Host entry
//! yet and is answered by no one here: it moves into the Host at F1 (E3.5
//! for the key adapter), never copied into a test.

use std::sync::{Arc, OnceLock, Weak};

use parking_lot::RwLock;
use rekindle_codec::community::envelope::{CommunityEnvelope, MekTransferPayload, SignedEnvelope};
use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::{Caller, ConnId};
use rekindle_node::host::adapters::governance::projection::GovernanceProjected;
use rekindle_node::host::{EndReason, Host, OwnRecordKeys, ShellHooks, UnlockedView};
use rekindle_transport::crypto::mek::{Mek, MekCache};
use rekindle_transport::operations::direct::{self, Called};
use rekindle_transport::payload::rpc::{CallResponse, InboundCall};
use rekindle_transport::{InboundHandler, Session, TransportEvent};
use rekindle_types::channel_keys::ChannelKeyProvider;

/// How a netsim node's Host runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimMode {
    /// The daemon's: the Host starts its transport (`TransportSource::Ports`).
    Daemon,
    /// The desktop's: the Host adopts a transport (`TransportSource::Adopted`).
    Shell,
}

impl SimMode {
    /// The caller a scenario request comes from on a node in this mode: on
    /// a daemon, a bus client with admin clearance, as the CLI on the same
    /// machine; on a shell, the window itself (`Caller::shell`), as the
    /// desktop calls its in-process Host, so the window's own identity
    /// create and unlock run (`daemon/dispatch/shell.rs` refuses them only
    /// to a bus client).
    pub fn caller(self) -> Caller {
        match self {
            Self::Shell => Caller::shell(),
            Self::Daemon => Caller {
                conn: ConnId::Bus(1),
                clearance: SecurityLevel::Admin,
                verified_name: None,
                static_key: None,
            },
        }
    }
}

/// The desktop shell's Veilid dispatch, reduced to the Host entries it calls.
pub struct ShellInbound {
    host: OnceLock<Weak<Host>>,
    clock: Arc<dyn rekindle_utils::Clock>,
    session: Arc<RwLock<Option<Session>>>,
}

impl ShellInbound {
    pub fn new(
        clock: Arc<dyn rekindle_utils::Clock>,
        session: Arc<RwLock<Option<Session>>>,
    ) -> Self {
        Self {
            host: OnceLock::new(),
            clock,
            session,
        }
    }

    /// The Host started over this transport.
    pub fn attach(&self, host: &Arc<Host>) {
        let _ = self.host.set(Arc::downgrade(host));
    }

    fn host(&self) -> Option<Arc<Host>> {
        self.host.get().and_then(Weak::upgrade)
    }

    /// The desktop's `MekTransfer` answer (`network.rs` `community_call`):
    /// the Host's receive path; a refusal is a bare `ACK`.
    fn mek_transfer(&self, transfer: &MekTransferPayload) -> CallResponse {
        let Some(host) = self.host() else {
            return CallResponse::Ok(b"ACK".to_vec());
        };
        CallResponse::Ok(
            host.governance()
                .receive_mek_transfer(transfer)
                .unwrap_or_else(|_| b"ACK".to_vec()),
        )
    }
}

impl InboundHandler for ShellInbound {
    fn local_identity(&self) -> Option<[u8; 32]> {
        let guard = self.session.read();
        let hex = &guard.as_ref()?.identity.public_key_hex;
        rekindle_transport::recipient_bytes(hex).ok()
    }

    fn clock(&self) -> &dyn rekindle_utils::Clock {
        self.clock.as_ref()
    }

    fn scope(&self) -> Arc<rekindle_lifecycle::SessionScope> {
        self.host()
            .and_then(|host| host.unlock_scope())
            .unwrap_or_else(|| rekindle_lifecycle::SessionScope::closed("no host"))
    }

    async fn on_direct(&self, datagram: &[u8]) {
        if let Some(host) = self.host() {
            host.receive_direct(datagram).await;
        }
    }

    async fn on_direct_call(&self, datagram: &[u8], route: Option<&str>) -> Option<Vec<u8>> {
        match self.host()?.receive_direct_call(datagram, route).await {
            Called::Answered(reply) => Some(reply),
            Called::RetryLater => Some(direct::RETRY_LATER.to_vec()),
            // The desktop answers these in `src-tauri` (F1).
            Called::Unported(_) => Some(direct::NAK.to_vec()),
        }
    }

    async fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        envelope: CommunityEnvelope,
        _lamport_ts: u64,
    ) {
        let (Some(host), Some(control)) = (self.host(), envelope.into_control()) else {
            return;
        };
        host.governance()
            .on_membership_control(community_id, sender_pseudonym, &control)
            .await;
    }

    async fn on_gossip_forward(&self, _envelope: &SignedEnvelope) {}

    async fn on_head_advanced(&self, community_id: &str, sender: &str, hint: &[u8]) -> bool {
        self.host().is_some_and(|host| {
            host.governance()
                .ingest_head_advanced(community_id, sender, hint)
        })
    }

    async fn on_call(&self, _sender: Option<&str>, request: InboundCall) -> CallResponse {
        match request {
            InboundCall::CommunityMekTransfer(transfer) => self.mek_transfer(&transfer),
            _ => CallResponse::Rejected {
                reason: "answered in src-tauri until F1".into(),
            },
        }
    }

    async fn on_value_change(&self, record: &str, subkeys: Vec<u32>, first: Option<Vec<u8>>) {
        let Some(host) = self.host() else {
            return;
        };
        if host.governance().on_watched_change(record, &subkeys).await {
            return;
        }
        host.friend_presence()
            .on_watched_change(record, &subkeys, first.as_deref())
            .await;
    }

    async fn on_event(&self, event: TransportEvent) {
        if let (
            Some(host),
            TransportEvent::AttachmentChanged {
                public_internet_ready,
                ..
            },
        ) = (self.host(), event)
        {
            host.friend_presence()
                .network_changed(public_internet_ready);
        }
    }
}

/// A shell that holds nothing but its one key cache: every stage succeeds.
/// The cache follows every key the Host installs
/// (`ShellHooks::on_mek_installed`), as the desktop's shell cache does
/// (`src-tauri/src/services/shell_session/governance.rs` `install_mek`):
/// on the shell mode the Host reads the keys it holds through
/// [`ShellHooks::channel_keys`] (`deps_impl.rs` `keys`).
pub struct BareShell {
    cache: Arc<RwLock<MekCache>>,
    keys: Arc<dyn ChannelKeyProvider>,
    /// The voice session and rosters (plan E3.4 S8-pre).
    pub session: Arc<crate::sim_session_presence::SimSessionPresence>,
}

impl Default for BareShell {
    fn default() -> Self {
        let cache = Arc::new(RwLock::new(MekCache::new()));
        Self {
            keys: rekindle_node::daemon::mek_rotation::provider_over(
                Arc::clone(&cache),
                rekindle_vault::new_handle(),
            ),
            cache,
            session: Arc::default(),
        }
    }
}

#[async_trait::async_trait]
impl ShellHooks for BareShell {
    async fn unlocked(
        &self,
        _view: &UnlockedView,
        _scope: &Arc<rekindle_lifecycle::SessionScope>,
    ) -> Result<(), String> {
        Ok(())
    }
    fn own_records(&self, _keys: &OwnRecordKeys) {}
    async fn ending(&self) {}
    async fn ended(&self, _reason: EndReason) {}
    fn own_gossip_signed(&self, _community_id: &str, _sender: &str, _dedup_key: &str) {}
    fn community_left(&self, _community_id: &str) {}
    fn spawn_departure_rotation(&self, _community_id: &str, _departed: &str) {}
    fn require_permission(&self, _community_id: &str, _perm_bits: u64) -> Result<(), String> {
        Ok(())
    }
    fn on_governance_projected(&self, _community_id: &str, _projected: &GovernanceProjected) {}
    fn spawn_community_mek_recovery(&self, _community_id: &str, _requester_pseudonym: &str) {}
    fn spawn_presence_poll_tick(&self, _community_id: &str) {}
    async fn presence_refresh(&self, _community_id: &str) {}
    fn push_onboarding_complete_to_sync(&self, _community_id: &str) {}
    fn on_mek_installed(
        &self,
        community_id: &str,
        scope: rekindle_types::channel_keys::KeyScope,
        key_bytes: [u8; 32],
        generation: u64,
    ) {
        self.cache
            .write()
            .insert(community_id, scope, Mek::from_bytes(key_bytes, generation));
    }
    fn channel_keys(&self) -> Arc<dyn ChannelKeyProvider> {
        Arc::clone(&self.keys)
    }

    fn session_presence(&self) -> Arc<dyn rekindle_presence::SessionPresence> {
        Arc::clone(&self.session) as Arc<dyn rekindle_presence::SessionPresence>
    }
    async fn community_records_ready(
        &self,
        _community_id: &str,
        _leases: rekindle_records::lease::CommunityLeases,
    ) {
    }
}
