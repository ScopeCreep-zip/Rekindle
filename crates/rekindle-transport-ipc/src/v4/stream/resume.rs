//! Stream resume protocol — eligibility tracking, evaluation, and cross-Session lineage.

use std::collections::HashMap;
use std::time::{Duration, Instant};

// ── ResumeConfig ──────────────────────────────────────────────────

/// Configuration for the resume protocol.
#[derive(Debug, Clone)]
pub struct ResumeConfig {
    /// How long a suspended Transfer remains eligible for resume.
    /// Default: 300 seconds (5 minutes). Petabyte workloads: up to 86400 (24 hours).
    pub eligibility_window: Duration,
}

impl Default for ResumeConfig {
    fn default() -> Self {
        Self {
            eligibility_window: Duration::from_secs(300),
        }
    }
}

impl ResumeConfig {
    pub fn with_window(eligibility_window: Duration) -> Self {
        Self { eligibility_window }
    }
}

// ── ResumeState ───────────────────────────────────────────────────

/// Per-suspended-Transfer state preserved for resume eligibility.
pub struct ResumeState {
    transfer_id: uuid::Uuid,
    resume_from_byte: u64,
    resume_from_chunk: u32,
    anchor_audit_link: [u8; 32],
    anchor_content_hash: [u8; 32],
    suspended_at: Instant,
    eligibility_window: Duration,
}

impl ResumeState {
    pub fn new(
        transfer_id: uuid::Uuid,
        resume_from_byte: u64,
        resume_from_chunk: u32,
        anchor_audit_link: [u8; 32],
        anchor_content_hash: [u8; 32],
        suspended_at: Instant,
        eligibility_window: Duration,
    ) -> Self {
        Self {
            transfer_id,
            resume_from_byte,
            resume_from_chunk,
            anchor_audit_link,
            anchor_content_hash,
            suspended_at,
            eligibility_window,
        }
    }

    pub fn transfer_id(&self) -> uuid::Uuid { self.transfer_id }
    pub fn resume_from_byte(&self) -> u64 { self.resume_from_byte }
    pub fn resume_from_chunk(&self) -> u32 { self.resume_from_chunk }
    pub fn anchor_audit_link(&self) -> &[u8; 32] { &self.anchor_audit_link }
    pub fn anchor_content_hash(&self) -> &[u8; 32] { &self.anchor_content_hash }

    pub fn is_expired(&self) -> bool {
        self.suspended_at.elapsed() >= self.eligibility_window
    }
}

// ── ResumeRegistry ────────────────────────────────────────────────

/// Maps TransferId → ResumeState for all suspended Transfers.
#[derive(Default)]
pub struct ResumeRegistry {
    entries: HashMap<uuid::Uuid, ResumeState>,
}

impl ResumeRegistry {
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    pub fn register(&mut self, state: ResumeState) {
        self.entries.insert(state.transfer_id, state);
    }

    pub fn lookup(&self, transfer_id: uuid::Uuid) -> Option<&ResumeState> {
        self.entries.get(&transfer_id)
    }

    pub fn remove(&mut self, transfer_id: uuid::Uuid) -> bool {
        self.entries.remove(&transfer_id).is_some()
    }

    /// Garbage-collect expired entries. Returns count removed.
    pub fn gc_expired(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, state| !state.is_expired());
        before - self.entries.len()
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

// ── Resume evaluation ─────────────────────────────────────────────

#[derive(Debug)]
pub enum ResumeDecision {
    Accepted { resume_from_chunk: u32 },
    Denied(ResumeDenialReason),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResumeDenialReason {
    TransferIdUnknown,
    ResumeWindowExpired,
    AuditAnchorMismatch,
    ContentAnchorMismatch,
}

/// Evaluate a STREAM_RESUME request against the registry.
///
/// Check order (determines which error surfaces first):
/// 1. TransferId exists in registry
/// 2. Eligibility window not expired
/// 3. Audit anchor matches
/// 4. Content hash anchor matches
pub fn evaluate_resume_request(
    registry: &ResumeRegistry,
    transfer_id: uuid::Uuid,
    claimed_audit_link: [u8; 32],
    claimed_content_hash: [u8; 32],
) -> ResumeDecision {
    let Some(state) = registry.lookup(transfer_id) else {
        return ResumeDecision::Denied(ResumeDenialReason::TransferIdUnknown);
    };

    if state.is_expired() {
        return ResumeDecision::Denied(ResumeDenialReason::ResumeWindowExpired);
    }

    if claimed_audit_link != state.anchor_audit_link {
        return ResumeDecision::Denied(ResumeDenialReason::AuditAnchorMismatch);
    }

    if claimed_content_hash != state.anchor_content_hash {
        return ResumeDecision::Denied(ResumeDenialReason::ContentAnchorMismatch);
    }

    ResumeDecision::Accepted {
        resume_from_chunk: state.resume_from_chunk,
    }
}

// ── Transfer Lineage ──────────────────────────────────────────────

/// One Session's contribution to a cross-Session Transfer.
#[derive(Debug, Clone)]
pub struct SessionAuditFragment {
    pub session_id: uuid::Uuid,
    pub anchor: [u8; 32],
    pub start_session_seq: u64,
    pub end_session_seq: u64,
    pub end_chain_link: [u8; 32],
}

/// Tracks a TransferId across multiple Sessions.
pub struct TransferLineage {
    transfer_id: uuid::Uuid,
    fragments: Vec<SessionAuditFragment>,
}

#[derive(Debug)]
pub enum LineageVerifyError {
    ContinuityBreak { fragment_index: usize },
}

impl TransferLineage {
    pub fn new(transfer_id: uuid::Uuid, initial_fragment: SessionAuditFragment) -> Self {
        Self {
            transfer_id,
            fragments: vec![initial_fragment],
        }
    }

    pub fn add_fragment(&mut self, fragment: SessionAuditFragment) {
        self.fragments.push(fragment);
    }

    pub fn transfer_id(&self) -> uuid::Uuid {
        self.transfer_id
    }

    pub fn fragment_count(&self) -> usize {
        self.fragments.len()
    }

    pub fn fragments(&self) -> &[SessionAuditFragment] {
        &self.fragments
    }

    /// Verify that each fragment's anchor matches the previous fragment's
    /// end_chain_link. Single-fragment lineage trivially succeeds.
    pub fn verify_continuity(&self) -> Result<(), LineageVerifyError> {
        for i in 1..self.fragments.len() {
            let prev_end = &self.fragments[i - 1].end_chain_link;
            let curr_anchor = &self.fragments[i].anchor;
            if prev_end != curr_anchor {
                return Err(LineageVerifyError::ContinuityBreak { fragment_index: i });
            }
        }
        Ok(())
    }
}
