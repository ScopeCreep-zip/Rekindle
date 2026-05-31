//! `MockState` — programmable behaviour + call recorders for the
//! shared community-presence test fixture. Extracted from the
//! `impl` block so the trait-impl file stays focused on dispatch.

use std::collections::{HashMap, HashSet};

use rekindle_protocol::dht::community::channel_record::ChannelMessage;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, SignedEnvelope};

use crate::community::GossipOverlaySnapshot;
use crate::deps::{OnlineMemberSnapshot, PresenceCredentials, SegmentDescriptor};

#[derive(Default)]
pub struct MockState {
    // ---- Read-side data ----
    pub my_pk: String,
    pub our_route: Option<Vec<u8>>,
    pub status: String,
    pub channels: Vec<String>,
    pub channel_logs: Vec<(String, String)>,
    pub member_count: u32,
    pub last_ts: HashMap<String, i64>,
    pub read_results: HashMap<String, Result<Vec<ChannelMessage>, String>>,
    // ---- Programmable behaviour ----
    pub registry_open_result: Option<String>,
    pub presence_credentials: Option<PresenceCredentials>,
    pub bans: HashSet<String>,
    pub segments: Vec<SegmentDescriptor>,
    pub segment_raw: HashMap<String, Vec<(u32, Vec<u8>)>>,
    pub member_roles: HashMap<String, Vec<u32>>,
    pub offline_diff: Vec<String>,
    pub gossip_snapshot: GossipOverlaySnapshot,
    pub stale_syncs: Vec<(String, u32)>,
    /// Peers the mock's `extend_online_with_recent_gossip` should
    /// inject into the orchestrator's `online_members` map — drives
    /// the `online_count > 0` gate that controls the stale-sync
    /// retry block.
    pub inject_online: HashMap<String, OnlineMemberSnapshot>,
    // ---- Call-recording side ----
    pub sent_envelopes: Vec<(String, CommunityEnvelope)>,
    pub pending_syncs: Vec<(String, String, u32)>,
    pub catchups: Vec<(String, String, usize)>,
    pub initial_done: Vec<String>,
    pub calls_my_pseudonym: Vec<String>,
    pub calls_status_str: Vec<String>,
    pub calls_channel_ids: Vec<String>,
    pub calls_channel_logs: Vec<String>,
    pub calls_member_count: Vec<String>,
    pub calls_last_ts: Vec<(String, String)>,
    pub calls_read_all: Vec<(String, u32)>,
    pub calls_self_snapshot: Vec<String>,
    pub calls_encrypt_history: Vec<(String, usize)>,
    pub calls_compute_history: Vec<String>,
    pub calls_sign_presence: Vec<(String, usize)>,
    pub calls_write_registry: Vec<(String, u32, usize, String)>,
    pub calls_persist_rows: Vec<(String, usize, usize, i64)>,
    pub calls_extend_known: Vec<(String, usize)>,
    pub calls_emit_discovered: Vec<(String, String, String, u32)>,
    pub calls_run_tick: Vec<String>,
    pub calls_install_shutdown: Vec<String>,
    pub calls_ensure_registry: Vec<String>,
    pub calls_presence_credentials: Vec<String>,
    pub calls_governance_bans: Vec<String>,
    pub calls_segment_descriptors: Vec<String>,
    pub calls_scan_segment: Vec<(String, u32, String, Option<u32>)>,
    pub calls_merge_roles: Vec<(String, usize, String)>,
    pub calls_apply_member_state: Vec<(String, usize, usize)>,
    pub calls_load_known_events: Vec<String>,
    pub calls_read_my_rsvps: Vec<String>,
    pub calls_write_rsvps: Vec<(String, usize)>,
    pub calls_read_profiles: Vec<String>,
    pub calls_apply_profiles: Vec<(String, usize, bool)>,
    pub calls_extend_online: Vec<(String, usize, String, u64)>,
    pub calls_offline_diff: Vec<(String, usize, String)>,
    pub calls_read_gossip: Vec<String>,
    pub calls_apply_gossip: Vec<(String, usize, usize)>,
    pub calls_send_raw: Vec<(String, SignedEnvelope)>,
    pub calls_emit_offline: Vec<(String, String)>,
    pub calls_stale_syncs: Vec<(String, u64, u64, u32)>,
    pub calls_update_pending: Vec<(String, String, u64, u32)>,
    pub calls_prune_pending: Vec<(String, u32)>,
    pub calls_auto_expand: Vec<String>,
}
