//! `MockState` — programmable behaviour + call recorders for the
//! shared community-presence test fixture. Extracted from the
//! `impl` block so the trait-impl file stays focused on dispatch.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::deps::{PresenceCredentials, RowWrite, SegmentDescriptor};

#[derive(Default)]
pub struct MockState {
    // ---- Read-side data ----
    pub my_pk: String,
    pub our_route: Option<Vec<u8>>,
    // ---- Programmable behaviour ----
    pub registry_open_result: Option<String>,
    pub presence_credentials: Option<PresenceCredentials>,
    pub bans: HashSet<String>,
    pub segments: Vec<SegmentDescriptor>,
    pub segment_raw: HashMap<String, Vec<(u32, Vec<u8>)>>,
    /// Outcomes `write_presence_to_registry_subkey` returns in order;
    /// `RowWrite::Stored` once exhausted.
    pub write_outcomes: VecDeque<RowWrite>,
    // ---- Call-recording side ----
    pub calls_my_pseudonym: Vec<String>,
    pub calls_self_snapshot: Vec<String>,
    pub calls_encrypt_history: Vec<(String, usize)>,
    pub calls_compute_history: Vec<String>,
    pub calls_sign_presence: Vec<(String, usize)>,
    pub calls_write_registry: Vec<(String, u32, usize, String)>,
    pub calls_persist_rows: Vec<(String, usize, usize, i64)>,
    pub calls_extend_known: Vec<(String, usize)>,
    pub calls_emit_discovered: Vec<(String, String, u32)>,
    pub calls_run_tick: Vec<String>,
    pub calls_ensure_registry: Vec<String>,
    pub calls_presence_credentials: Vec<String>,
    pub calls_governance_bans: Vec<String>,
    pub calls_segment_descriptors: Vec<String>,
    pub calls_scan_segment: Vec<(String, u32, String, Option<u32>)>,
    pub calls_apply_member_state: Vec<(String, usize)>,
    pub calls_read_profiles: Vec<String>,
    pub calls_apply_profiles: Vec<(String, usize, bool)>,
    pub calls_auto_expand: Vec<String>,
    pub active_voice_channel: Option<String>,
    pub calls_reconcile_voice_roster: Vec<(String, Vec<crate::deps::VoicePresenceRow>)>,
}
