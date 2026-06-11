//! TrustStore — sharded in-memory registry of per-peer trust records.
//!
//! Three internal DashMaps keyed by `IdentityRoot`:
//! - `records`: anchor root → TrustRecord (primary, key never changes)
//! - `head_to_anchor`: current-head root → anchor root (secondary index)
//! - `prior_head`: anchor root → prior head root (eviction tracking)
//!
//! **Lock architecture:**
//! - `invariant_lock: RwLock<()>` — protects the cross-map invariant.
//!   Write side: all mutation paths. Read side: `snapshot_by_root`.
//! - `upgrade_mutex: Mutex<()>` — serializes the read→write upgrade in
//!   `resolve_or_observe`. Without it, two threads that both miss the
//!   read path race for the write lock and the loser may starve under
//!   unfair `RwLock` implementations (per matrix-rust-sdk `StateLock` pattern).
//! - Lock-free: `state`, `snapshot`, `is_within_grace` — these take
//!   `&PeerRef` and go straight to `records` via `peer.root()`.
//!
//! **Double-checked locking in `resolve_or_observe`:**
//! Read lock → try Arm 1 (index) and Arm 2 (primary key) → on hit, return.
//! On miss → drop read → acquire upgrade_mutex → acquire write lock →
//! re-check both arms (a concurrent rotation or first-contact may have
//! landed) → only then Arm 3 (first contact). The re-check is mandatory:
//! without it, the split-identity race returns.
//!
//! **Provisional identity reconciliation:** when `apply_rotation`
//! advances a peer to `new_head`, it checks whether `new_head` already
//! exists as a primary key (a provisional first-contact record created
//! before the rotation proof arrived). If so, it merges the provisional
//! into the anchor and removes the orphan.
//!
//! Merge rule (all paths through `transition()`):
//! - `previously_verified` latch: always OR'd (latches never clear).
//! - `Verified` on provisional: `transition(VerificationSucceeded)`.
//! - `VerificationViolation`/`PinViolation`: `transition(UnexplainedRootChange)`
//!   with merged latch. The table produces VerificationViolation if latch set.
//! - `Pinned`: no state change (nothing to transfer).

use std::sync::{Mutex, RwLock};

use dashmap::DashMap;

use crate::error::IdentityError;
use crate::origin::originate::{IdentityRoot, RevocationCertificate};
use crate::root::PeerRef;
use crate::root::rotation::RotationChain;
use crate::root::termination::DeathNotice;
use crate::root::ROTATION_GRACE;
use crate::wire::signable::Hlc;
use crate::wire::verified::Verified;

use super::record::{TrustRecord, TrustRecordPersist};
use super::state::{TrustEvent, TrustState, transition};

/// A change in trust state worth surfacing to the user.
/// Only "significant" changes are emitted — not every transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityStatusChange {
    pub peer: PeerRef,
    pub old_state: TrustState,
    pub new_state: TrustState,
}

impl IdentityStatusChange {
    /// Whether this change is "significant" — should surface a UI warning.
    /// Per matrix-rust-sdk RoomIdentityState: significant changes are
    /// violations and their resolution. Insignificant: Pinned→Verified.
    pub fn is_significant(&self) -> bool {
        use TrustState::*;
        !matches!(
            (self.old_state, self.new_state),
            (Pinned, Verified) | (Verified, Pinned)
        ) && self.old_state != self.new_state
    }
}

pub struct TrustStore {
    records: DashMap<IdentityRoot, TrustRecord>,
    /// Forward index: head root → anchor root.
    head_to_anchor: DashMap<IdentityRoot, IdentityRoot>,
    /// Reverse index: anchor root → all head roots pointing to it.
    /// Used by `apply_death` for O(1) cleanup instead of O(n) retain().
    anchor_to_heads: DashMap<IdentityRoot, Vec<IdentityRoot>>,
    /// Tracks the immediately-prior head per anchor for index eviction.
    prior_head: DashMap<IdentityRoot, IdentityRoot>,
    /// Roots declared dead. Blocks re-observation.
    dead_roots: DashMap<IdentityRoot, ()>,
    /// Write side: all mutation paths.
    /// Read side: `snapshot_by_root` (index-touching read).
    invariant_lock: RwLock<()>,
    /// Serializes the read→write upgrade in `resolve_or_observe`.
    upgrade_mutex: Mutex<()>,
    /// Event sender for trust state changes. Subscribers receive only
    /// significant changes. `None` if no subscriber.
    event_tx: Mutex<Option<std::sync::mpsc::Sender<IdentityStatusChange>>>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self {
            records: DashMap::new(),
            head_to_anchor: DashMap::new(),
            anchor_to_heads: DashMap::new(),
            prior_head: DashMap::new(),
            dead_roots: DashMap::new(),
            invariant_lock: RwLock::new(()),
            upgrade_mutex: Mutex::new(()),
            event_tx: Mutex::new(None),
        }
    }

    /// Subscribe to significant trust state changes. Returns the receiving end.
    /// Only one subscriber at a time — calling again replaces the previous.
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<IdentityStatusChange> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.event_tx.lock().expect("event_tx poisoned") = Some(tx);
        rx
    }

    /// Emit a state change event if significant.
    fn emit_if_significant(&self, peer: PeerRef, old_state: TrustState, new_state: TrustState) {
        if old_state == new_state { return; }
        let change = IdentityStatusChange { peer, old_state, new_state };
        if change.is_significant() {
            if let Some(tx) = self.event_tx.lock().expect("event_tx poisoned").as_ref() {
                let _ = tx.send(change);
            }
        }
    }

    // ── The single external door ────────────────────────────────

    /// Given an observed root, returns the stable `PeerRef` and current
    /// trust state. Handles known-head, known-anchor, and first-contact.
    ///
    /// Double-checked locking: read lock for the fast path (Arms 1/2),
    /// upgrade to write lock with re-check for the slow path (Arm 3).
    pub fn resolve_or_observe(&self, root: IdentityRoot) -> (PeerRef, TrustState) {
        // Fast path: read lock. Arms 1 and 2 are pure reads.
        {
            let _rguard = self.invariant_lock.read().expect("lock poisoned");

            if let Some(anchor_root) = self.head_to_anchor_get(&root) {
                tracing::trace!(root = ?&root.as_bytes()[..4], "resolve: Arm 1 (read) head in index");
                let state = self.records.get(&anchor_root)
                    .map(|r| r.state)
                    .unwrap_or(TrustState::Pinned);
                return (PeerRef::anchor(anchor_root), state);
            }
            if let Some(entry) = self.records.get(&root) {
                tracing::trace!(root = ?&root.as_bytes()[..4], "resolve: Arm 2 (read) primary key");
                return (PeerRef::anchor(root), entry.state);
            }
            tracing::trace!(root = ?&root.as_bytes()[..4], "resolve: read miss, upgrading");
        } // ← read guard dropped

        // Slow path: serialize the upgrade, then write lock with re-check.
        let _upgrade = self.upgrade_mutex.lock().expect("upgrade mutex poisoned");
        let _wguard = self.invariant_lock.write().expect("lock poisoned");

        // Re-check Arm 1 under write lock — a concurrent rotation may have landed.
        if let Some(anchor_root) = self.head_to_anchor_get(&root) {
            tracing::trace!(root = ?&root.as_bytes()[..4], "resolve: Arm 1 (write re-check) head in index");
            let state = self.records.get(&anchor_root)
                .map(|r| r.state)
                .unwrap_or(TrustState::Pinned);
            return (PeerRef::anchor(anchor_root), state);
        }
        // Re-check Arm 2 under write lock — a concurrent first-contact may have landed.
        if let Some(entry) = self.records.get(&root) {
            tracing::trace!(root = ?&root.as_bytes()[..4], "resolve: Arm 2 (write re-check) primary key");
            return (PeerRef::anchor(root), entry.state);
        }

        // Arm 3: genuine first contact, write lock held.
        tracing::debug!(root = ?&root.as_bytes()[..4], "resolve: Arm 3 first contact");
        self.observe_root_internal(root)
    }

    // ── Internal helpers ────────────────────────────────────────

    fn head_to_anchor_get(&self, root: &IdentityRoot) -> Option<IdentityRoot> {
        self.head_to_anchor.get(root).map(|e| *e.value())
    }

    /// Insert into the forward index and maintain the reverse index.
    fn index_insert(&self, head: IdentityRoot, anchor: IdentityRoot) {
        self.head_to_anchor.insert(head, anchor);
        self.anchor_to_heads.entry(anchor).or_default().push(head);
    }

    /// Remove a single head from both indexes.
    fn index_remove_head(&self, head: &IdentityRoot) {
        if let Some((_, anchor)) = self.head_to_anchor.remove(head) {
            if let Some(mut heads) = self.anchor_to_heads.get_mut(&anchor) {
                heads.retain(|h| h != head);
            }
        }
    }

    /// Remove ALL heads for an anchor from both indexes. O(k) where k is
    /// the number of heads for this anchor (bounded to 2 by eviction).
    fn index_remove_all_for_anchor(&self, anchor: &IdentityRoot) {
        if let Some((_, heads)) = self.anchor_to_heads.remove(anchor) {
            for head in heads {
                self.head_to_anchor.remove(&head);
            }
        }
    }

    fn resolve_anchor_root(&self, root: &IdentityRoot) -> Option<IdentityRoot> {
        if let Some(anchor) = self.head_to_anchor_get(root) {
            return Some(anchor);
        }
        if self.records.contains_key(root) {
            return Some(*root);
        }
        None
    }

    fn observe_root_internal(&self, root: IdentityRoot) -> (PeerRef, TrustState) {
        // Block re-observation of dead roots. A dead identity must not
        // be resurrected by a network observation.
        if self.dead_roots.contains_key(&root) {
            tracing::debug!(root = ?&root.as_bytes()[..4], "first contact blocked: root is dead");
            // Return a synthetic Pinned state — the caller gets a PeerRef
            // but any subsequent operation will find no record and fail.
            // This is the correct behavior: the peer is known-dead, not unknown.
            return (PeerRef::anchor(root), TrustState::Pinned);
        }

        let entry = self.records.entry(root);
        match entry {
            dashmap::mapref::entry::Entry::Vacant(v) => {
                tracing::debug!(root = ?&root.as_bytes()[..4], "first contact: inserting");
                let record = TrustRecord::first_contact(root);
                let state = record.state;
                v.insert(record);
                (PeerRef::anchor(root), state)
            }
            dashmap::mapref::entry::Entry::Occupied(o) => {
                // DashMap::entry contract: occupied key equals `root`.
                (PeerRef::anchor(root), o.get().state)
            }
        }
    }

    // ── Rotation with provisional reconciliation ────────────────

    /// Apply a verified rotation chain. Advances `pinned_root`, opens
    /// the grace window, updates the secondary index.
    ///
    /// **Provisional reconciliation:** if `new_head` already exists as
    /// a primary key, the provisional record is merged into the anchor
    /// and the orphan is removed. See module doc for merge rule.
    pub fn apply_rotation(
        &self,
        peer: &PeerRef,
        chain: &RotationChain,
        now: Hlc,
    ) -> Result<TrustState, IdentityError> {
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        let anchor_root = *peer.root();

        // Phase 1: read+mutate the anchor entry, then drop the guard.
        // This prevents the DashMap shard deadlock where get_mut(anchor) and
        // remove(new_head) hash to the same shard.
        tracing::debug!(anchor = ?&anchor_root.as_bytes()[..4], "rotation: phase 1");
        let (old_head, new_head, anchor_state, anchor_latch, pre_rotation_state) = {
            let mut entry = self.records.get_mut(&anchor_root)
                .ok_or(IdentityError::PeerNotInStore)?;
            let pre_rotation_state = entry.state;

            let old_head = entry.pinned_root;
            let (new_head, new_epoch) = chain.head();
            let grace_until = Hlc::new(
                now.physical_ns + ROTATION_GRACE.as_nanos() as u64,
                0,
            );

            entry.advance_rotation(new_head, new_epoch, grace_until, chain.len() as u64);

            let (new_state, new_latch) = transition(
                entry.state,
                entry.previously_verified,
                TrustEvent::RotationProofVerified,
            );
            entry.state = new_state;
            entry.previously_verified = new_latch;

            (old_head, new_head, new_state, new_latch, pre_rotation_state)
        }; // ← guard dropped, shard lock released

        // Phase 2: provisional reconciliation (no anchor guard held).
        let mut final_state = anchor_state;
        if new_head != anchor_root {
            if let Some((_, provisional)) = self.records.remove(&new_head) {
                tracing::debug!(
                    anchor = ?&anchor_root.as_bytes()[..4],
                    new_head = ?&new_head.as_bytes()[..4],
                    prov_state = ?provisional.state,
                    prov_latch = provisional.previously_verified,
                    "rotation: merging provisional"
                );

                let mut merged_state = anchor_state;
                let mut merged_latch = anchor_latch;

                // Latch always OR'd — latches never clear.
                if provisional.previously_verified {
                    merged_latch = true;
                }

                match provisional.state {
                    TrustState::Verified => {
                        let (s, l) = transition(merged_state, merged_latch,
                            TrustEvent::VerificationSucceeded);
                        merged_state = s;
                        merged_latch = l;
                    }
                    TrustState::VerificationViolation | TrustState::PinViolation => {
                        let (s, l) = transition(merged_state, merged_latch,
                            TrustEvent::UnexplainedRootChange);
                        merged_state = s;
                        merged_latch = l;
                    }
                    TrustState::Pinned => {}
                }

                // Phase 3: re-acquire anchor to write merged state.
                let mut entry = self.records.get_mut(&anchor_root)
                    .expect("anchor entry vanished under write lock — invariant violation");
                entry.state = merged_state;
                entry.previously_verified = merged_latch;
                tracing::debug!(
                    anchor = ?&anchor_root.as_bytes()[..4],
                    merged_state = ?merged_state,
                    merged_latch,
                    "rotation: merge written"
                );
                drop(entry);

                // Clean provisional's index entries
                self.index_remove_all_for_anchor(&new_head);
                self.prior_head.remove(&new_head);

                final_state = merged_state;
            }
        }

        // Phase 4: index maintenance.
        if let Some((_, prior_prior)) = self.prior_head.remove(&anchor_root) {
            if prior_prior != anchor_root {
                self.index_remove_head(&prior_prior);
            }
        }
        if old_head != anchor_root {
            self.prior_head.insert(anchor_root, old_head);
        }
        self.index_insert(new_head, anchor_root);
        tracing::debug!(
            anchor = ?&anchor_root.as_bytes()[..4],
            new_head = ?&new_head.as_bytes()[..4],
            final_state = ?final_state,
            "rotation: complete"
        );

        self.emit_if_significant(*peer, pre_rotation_state, final_state);
        Ok(final_state)
    }

    // ── Revocation ──────────────────────────────────────────────

    pub fn apply_revocation(
        &self,
        cert: &Verified<RevocationCertificate>,
    ) -> Result<TrustState, IdentityError> {
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        let cert = cert.get();
        tracing::debug!(root = ?&cert.root.as_bytes()[..4], "revocation");

        let anchor_root = self.resolve_anchor_root(&cert.root)
            .ok_or(IdentityError::PeerNotInStore)?;

        let mut entry = self.records.get_mut(&anchor_root)
            .ok_or(IdentityError::RecordDisappeared)?;

        entry.void_grace();
        entry.revoked_at_epoch = Some(cert.epoch_at_issue);

        let (new_state, new_latch) = transition(
            entry.state,
            entry.previously_verified,
            TrustEvent::RevocationReceived,
        );
        entry.state = new_state;
        entry.previously_verified = new_latch;

        Ok(new_state)
    }

    // ── Death ───────────────────────────────────────────────────

    pub fn apply_death(
        &self,
        notice: &Verified<DeathNotice>,
    ) -> Option<TrustRecord> {
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        let notice = notice.get();
        tracing::debug!(root = ?&notice.root.as_bytes()[..4], "death");

        let anchor_root = self.resolve_anchor_root(&notice.root)?;

        let removed = self.records.remove(&anchor_root).map(|(_, record)| record);
        self.index_remove_all_for_anchor(&anchor_root);
        self.prior_head.remove(&anchor_root);

        // Block future re-observation of this root and any heads that
        // pointed to it. A dead identity must not be resurrected.
        self.dead_roots.insert(anchor_root, ());
        self.dead_roots.insert(notice.root, ());

        removed
    }

    // ── Verification / acknowledgement ──────────────────────────

    pub fn mark_verified(
        &self,
        peer: &PeerRef,
    ) -> Result<TrustState, IdentityError> {
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        tracing::debug!(root = ?&peer.root().as_bytes()[..4], "mark_verified");

        let mut entry = self.records.get_mut(peer.root())
            .ok_or(IdentityError::PeerNotInStore)?;

        let old_state = entry.state;
        let (new_state, new_latch) = transition(
            entry.state,
            entry.previously_verified,
            TrustEvent::VerificationSucceeded,
        );
        entry.state = new_state;
        entry.previously_verified = new_latch;
        drop(entry);

        self.emit_if_significant(*peer, old_state, new_state);
        Ok(new_state)
    }

    /// Pin the current root — resolves PinViolation without affecting
    /// the previously_verified latch. The user acknowledges the identity
    /// change without withdrawing their verification requirement.
    ///
    /// Per matrix-rust-sdk: `pin_current_master_key()` resolves pin
    /// violation only. It does NOT clear `previously_verified`.
    pub fn pin_current_root(
        &self,
        peer: &PeerRef,
    ) -> Result<TrustState, IdentityError> {
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        tracing::debug!(root = ?&peer.root().as_bytes()[..4], "pin_current_root");

        let mut entry = self.records.get_mut(peer.root())
            .ok_or(IdentityError::PeerNotInStore)?;

        let old_state = entry.state;
        entry.pinned_root = entry.anchor_root;

        let (new_state, new_latch) = transition(
            entry.state,
            entry.previously_verified,
            TrustEvent::UserAcknowledged,
        );
        entry.state = new_state;
        entry.previously_verified = new_latch;
        drop(entry);

        self.emit_if_significant(*peer, old_state, new_state);
        Ok(new_state)
    }

    /// Withdraw verification — resolves VerificationViolation AND clears
    /// the previously_verified latch. The user is saying "I no longer hold
    /// this peer to the verified standard."
    ///
    /// Per matrix-rust-sdk: `withdraw_verification()` calls `pin()` first
    /// (update pinned_root), then clears the latch. Future unexplained
    /// changes will produce PinViolation, not VerificationViolation.
    pub fn withdraw_verification(
        &self,
        peer: &PeerRef,
    ) -> Result<TrustState, IdentityError> {
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        tracing::debug!(root = ?&peer.root().as_bytes()[..4], "withdraw_verification");

        let mut entry = self.records.get_mut(peer.root())
            .ok_or(IdentityError::PeerNotInStore)?;

        let old_state = entry.state;
        entry.pinned_root = entry.anchor_root;

        let (new_state, new_latch) = transition(
            entry.state,
            entry.previously_verified,
            TrustEvent::VerificationWithdrawn,
        );
        entry.state = new_state;
        entry.previously_verified = new_latch;
        drop(entry);

        self.emit_if_significant(*peer, old_state, new_state);
        Ok(new_state)
    }

    /// Cascade own trust change: when our own identity becomes verified,
    /// set `previously_verified = true` on all peers signed by our identity.
    /// Per matrix-rust-sdk: `check_all_identities_and_update_was_previously_
    /// verified_flag_if_needed`.
    ///
    /// `is_signed_by_us` is a caller-provided predicate. The trust store
    /// does not know how to verify signatures — that logic lives in the
    /// chat layer.
    pub fn cascade_own_trust_change(
        &self,
        is_signed_by_us: impl Fn(&IdentityRoot) -> bool,
    ) -> usize {
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        tracing::debug!("cascade_own_trust_change: scanning all records");

        let mut updated = 0;
        for mut entry in self.records.iter_mut() {
            if !entry.previously_verified && is_signed_by_us(&entry.anchor_root) {
                tracing::debug!(
                    root = ?&entry.anchor_root.as_bytes()[..4],
                    "cascade: setting previously_verified latch"
                );
                entry.previously_verified = true;
                updated += 1;
            }
        }
        tracing::debug!(updated, "cascade_own_trust_change: complete");
        updated
    }

    /// Handle an unexplained root change (no rotation proof).
    ///
    /// Updates `pinned_root` and trips the state machine. Does NOT
    /// write to the secondary index — an unproven root has no
    /// continuity claim. `pinned_root` after this call is display-only
    /// and deliberately non-resolvable through the index.
    pub fn unexplained_root_change(
        &self,
        peer: &PeerRef,
        new_root: IdentityRoot,
    ) -> Result<TrustState, IdentityError> {
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        tracing::debug!(
            root = ?&peer.root().as_bytes()[..4],
            new_root = ?&new_root.as_bytes()[..4],
            "unexplained_root_change"
        );

        let mut entry = self.records.get_mut(peer.root())
            .ok_or(IdentityError::PeerNotInStore)?;

        let old_state = entry.state;
        let (new_state, new_latch) = transition(
            entry.state,
            entry.previously_verified,
            TrustEvent::UnexplainedRootChange,
        );
        entry.state = new_state;
        entry.previously_verified = new_latch;
        entry.pinned_root = new_root;
        entry.void_grace();
        drop(entry);

        self.emit_if_significant(*peer, old_state, new_state);
        Ok(new_state)
    }

    // ── Read operations ─────────────────────────────────────────

    /// Lock-free: goes straight to primary map via anchor root.
    pub fn snapshot(&self, peer: &PeerRef) -> Option<TrustRecord> {
        self.records.get(peer.root()).map(|entry| entry.clone())
    }

    /// Takes the read lock: touches the index to resolve head → anchor.
    pub fn snapshot_by_root(&self, root: &IdentityRoot) -> Option<TrustRecord> {
        let _guard = self.invariant_lock.read().expect("lock poisoned");
        let anchor = if let Some(a) = self.head_to_anchor_get(root) {
            a
        } else if self.records.contains_key(root) {
            *root
        } else {
            return None;
        };
        self.records.get(&anchor).map(|entry| entry.clone())
    }

    /// Lock-free: direct primary key lookup.
    pub fn state(&self, peer: &PeerRef) -> Option<TrustState> {
        self.records.get(peer.root()).map(|entry| entry.state)
    }

    /// Lock-free: direct primary key lookup.
    pub fn is_within_grace(&self, peer: &PeerRef, now: &Hlc) -> bool {
        self.records.get(peer.root()).is_some_and(|entry| entry.is_within_grace(now))
    }

    pub fn len(&self) -> usize { self.records.len() }
    pub fn is_empty(&self) -> bool { self.records.is_empty() }

    // ── Persistence ─────────────────────────────────────────────

    pub fn export(&self) -> Vec<TrustRecordPersist> {
        self.records.iter().map(|entry| entry.value().clone()).collect()
    }

    pub fn import(records: Vec<TrustRecordPersist>) -> Self {
        let store = Self::new();
        for record in records {
            let anchor = record.anchor_root;
            // Rebuild head_to_anchor + anchor_to_heads for current head
            if record.pinned_root != anchor {
                store.index_insert(record.pinned_root, anchor);
            }
            // Rebuild prior_head from persisted prior_head_root
            if let Some(prior) = record.prior_head_root {
                if prior != anchor {
                    store.prior_head.insert(anchor, prior);
                    // Prior head also needs to be in the forward+reverse index
                    store.index_insert(prior, anchor);
                }
            }
            store.records.insert(anchor, record);
        }
        store
    }
}

impl Default for TrustStore {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::originate::{originate_from_seed, RotationEpoch};
    use crate::origin::seed::OriginSeed;
    use crate::root::rotation::{RotationChain, RotationProof};
    use crate::root::termination::DeathNotice;
    use zeroize::Zeroizing;

    fn make_root(byte: u8) -> (IdentityRoot, [u8; 32]) {
        let seed = [byte; 32];
        let o = originate_from_seed(OriginSeed::from_vault_bytes(Zeroizing::new(seed))).unwrap();
        (o.root, seed)
    }

    #[test]
    fn first_contact() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        let (peer, state) = store.resolve_or_observe(root);
        assert_eq!(state, TrustState::Pinned);
        assert_eq!(*peer.root(), root);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn same_root_twice() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        let (p1, _) = store.resolve_or_observe(root);
        let (p2, _) = store.resolve_or_observe(root);
        assert_eq!(p1, p2);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn verify_sets_latch() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        let (peer, _) = store.resolve_or_observe(root);
        let state = store.mark_verified(&peer).unwrap();
        assert_eq!(state, TrustState::Verified);
        let record = store.snapshot(&peer).unwrap();
        assert!(record.previously_verified);
    }

    #[test]
    fn unexplained_change_with_latch() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        let (new_root, _) = make_root(0x02);
        let (peer, _) = store.resolve_or_observe(root);
        store.mark_verified(&peer).unwrap();
        let state = store.unexplained_root_change(&peer, new_root).unwrap();
        assert_eq!(state, TrustState::VerificationViolation);
        assert!(store.head_to_anchor_get(&new_root).is_none(),
            "unexplained root must not be indexed");
        let record = store.snapshot(&peer).unwrap();
        assert_eq!(record.pinned_root, new_root);
    }

    #[test]
    fn unexplained_change_without_latch() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        let (new_root, _) = make_root(0x02);
        let (peer, _) = store.resolve_or_observe(root);
        let state = store.unexplained_root_change(&peer, new_root).unwrap();
        assert_eq!(state, TrustState::PinViolation);
    }

    #[test]
    fn rotation_opens_grace_anchor_stable() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (anchor, _) = store.resolve_or_observe(root_0);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();

        let now = Hlc::now();
        store.apply_rotation(&anchor, &chain, now).unwrap();

        assert!(store.is_within_grace(&anchor, &now));
        let record = store.snapshot(&anchor).unwrap();
        assert_eq!(record.pinned_root, root_1);

        let (resolved, _) = store.resolve_or_observe(root_1);
        assert_eq!(resolved, anchor);
        let (anchor_again, _) = store.resolve_or_observe(root_0);
        assert_eq!(anchor_again, anchor);

        let record_via_head = store.snapshot_by_root(&root_1).unwrap();
        assert_eq!(record_via_head.pinned_root, root_1);
    }

    #[test]
    fn rotation_merges_provisional_identity() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);

        let (anchor, _) = store.resolve_or_observe(root_0);
        let (provisional_peer, _) = store.resolve_or_observe(root_1);
        assert_ne!(anchor, provisional_peer, "before merge: two separate peers");
        assert_eq!(store.len(), 2, "before merge: two primary entries");

        store.mark_verified(&provisional_peer).unwrap();
        let provisional_record = store.snapshot(&provisional_peer).unwrap();
        assert!(provisional_record.previously_verified);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        store.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();

        assert_eq!(store.len(), 1, "after merge: provisional orphan removed");

        let (resolved, _) = store.resolve_or_observe(root_1);
        assert_eq!(resolved, anchor, "root_1 must resolve to root_0 anchor");

        let merged_record = store.snapshot(&anchor).unwrap();
        assert!(merged_record.previously_verified,
            "provisional verification must survive merge");
        assert_eq!(merged_record.pinned_root, root_1,
            "pinned_root must be the rotation proof's head");
    }

    #[test]
    fn merge_provisional_verification_violation_transfers_as_pin_violation() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (root_change, _) = make_root(0x03);

        let (anchor, _) = store.resolve_or_observe(root_0);
        let (provisional, _) = store.resolve_or_observe(root_1);

        store.mark_verified(&provisional).unwrap();
        store.unexplained_root_change(&provisional, root_change).unwrap();

        let prov_record = store.snapshot(&provisional).unwrap();
        assert_eq!(prov_record.state, TrustState::VerificationViolation);
        assert!(prov_record.previously_verified);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        store.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();

        assert_eq!(store.len(), 1, "provisional merged");

        let merged = store.snapshot(&anchor).unwrap();
        assert!(merged.previously_verified,
            "latch must survive merge from VerificationViolation provisional");
        assert_eq!(merged.state, TrustState::VerificationViolation,
            "latch=true + unexplained hop = VerificationViolation per transition table");
        assert_eq!(merged.pinned_root, root_1,
            "pinned_root must be the rotation proof's head, not root_change");
    }

    #[test]
    fn merge_provisional_pin_violation_transfers() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (root_change, _) = make_root(0x03);

        let (anchor, _) = store.resolve_or_observe(root_0);
        let (provisional, _) = store.resolve_or_observe(root_1);

        store.unexplained_root_change(&provisional, root_change).unwrap();
        let prov_record = store.snapshot(&provisional).unwrap();
        assert_eq!(prov_record.state, TrustState::PinViolation);
        assert!(!prov_record.previously_verified);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        store.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();

        assert_eq!(store.len(), 1, "provisional merged");

        let merged = store.snapshot(&anchor).unwrap();
        assert!(!merged.previously_verified);
        assert_eq!(merged.state, TrustState::PinViolation,
            "PinViolation on provisional must transfer — the unexplained hop is still unexplained");
        assert_eq!(merged.pinned_root, root_1,
            "pinned_root must be the rotation proof's head, not root_change");
    }

    #[test]
    fn revocation_voids_grace_via_index() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (anchor, _) = store.resolve_or_observe(root_0);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        let now = Hlc::now();
        store.apply_rotation(&anchor, &chain, now).unwrap();

        let cert = RevocationCertificate {
            root: root_1,
            epoch_at_issue: RotationEpoch::ORIGIN,
            signature: crate::wire::signable::Signature64::ZERO,
        };
        let verified_cert = Verified::new_for_test(cert);
        store.apply_revocation(&verified_cert).unwrap();

        assert!(!store.is_within_grace(&anchor, &now));
        let record = store.snapshot(&anchor).unwrap();
        assert_eq!(record.revoked_at_epoch, Some(RotationEpoch::ORIGIN));
    }

    #[test]
    fn death_removes_record() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        let (anchor, _) = store.resolve_or_observe(root);

        let death = DeathNotice {
            root,
            epoch: RotationEpoch::ORIGIN,
            issued_at: Hlc::now(),
            signature: crate::wire::signable::Signature64::ZERO,
        };
        let verified_death = Verified::new_for_test(death);
        let removed = store.apply_death(&verified_death);
        assert!(removed.is_some());
        assert_eq!(store.len(), 0);
        assert!(store.snapshot(&anchor).is_none());
    }

    #[test]
    fn death_via_index_after_rotation() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (anchor, _) = store.resolve_or_observe(root_0);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        store.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();

        let death = DeathNotice {
            root: root_1,
            epoch: RotationEpoch(1),
            issued_at: Hlc::now(),
            signature: crate::wire::signable::Signature64::ZERO,
        };
        let verified_death = Verified::new_for_test(death);
        let removed = store.apply_death(&verified_death);
        assert!(removed.is_some());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn index_bounded_two_genuine_heads() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, seed_1) = make_root(0x02);
        let (root_2, _) = make_root(0x03);
        let (anchor, _) = store.resolve_or_observe(root_0);

        assert!(store.head_to_anchor_get(&root_0).is_none(),
            "anchor must not be in the head index");

        let p1 = RotationProof::create(&seed_0, root_0, root_1, RotationEpoch(1), Hlc::now()).unwrap();
        let c1 = RotationChain::verify((root_0, RotationEpoch::ORIGIN), vec![p1]).unwrap();
        store.apply_rotation(&anchor, &c1, Hlc::now()).unwrap();

        assert!(store.head_to_anchor_get(&root_1).is_some());
        assert!(store.snapshot_by_root(&root_0).is_some(), "anchor always resolves");
        assert!(store.snapshot_by_root(&root_1).is_some(), "current head resolves");

        let p2 = RotationProof::create(&seed_1, root_1, root_2, RotationEpoch(2), Hlc::now()).unwrap();
        let c2 = RotationChain::verify((root_1, RotationEpoch(1)), vec![p2]).unwrap();
        store.apply_rotation(&anchor, &c2, Hlc::now()).unwrap();

        assert!(store.snapshot_by_root(&root_2).is_some(), "current head resolves");
        assert!(store.snapshot_by_root(&root_1).is_some(), "prior head resolves");
        assert!(store.snapshot_by_root(&root_0).is_some(), "anchor always resolves");

        let (root_3, _) = make_root(0x04);
        let seed_2 = make_root(0x03).1;
        let p3 = RotationProof::create(&seed_2, root_2, root_3, RotationEpoch(3), Hlc::now()).unwrap();
        let c3 = RotationChain::verify((root_2, RotationEpoch(2)), vec![p3]).unwrap();
        store.apply_rotation(&anchor, &c3, Hlc::now()).unwrap();

        assert!(store.snapshot_by_root(&root_3).is_some(), "current head resolves");
        assert!(store.snapshot_by_root(&root_2).is_some(), "prior head resolves");
        assert!(store.head_to_anchor_get(&root_1).is_none(),
            "prior-prior head must be evicted from index");
        assert!(store.snapshot_by_root(&root_0).is_some(), "anchor always resolves");
    }

    #[test]
    fn export_import_roundtrip() {
        let store = TrustStore::new();
        let (root_a, _) = make_root(0x01);
        let (root_b, _) = make_root(0x02);
        let (peer_a, _) = store.resolve_or_observe(root_a);
        store.resolve_or_observe(root_b);
        store.mark_verified(&peer_a).unwrap();

        let exported = store.export();
        assert_eq!(exported.len(), 2);

        let restored = TrustStore::import(exported);
        assert_eq!(restored.len(), 2);

        let (peer_a_restored, _) = restored.resolve_or_observe(root_a);
        let record_a = restored.snapshot(&peer_a_restored).unwrap();
        assert_eq!(record_a.state, TrustState::Verified);
        assert!(record_a.previously_verified);
    }

    #[test]
    fn import_rebuilds_secondary_index() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (anchor, _) = store.resolve_or_observe(root_0);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        store.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();

        let exported = store.export();
        let restored = TrustStore::import(exported);

        assert!(restored.snapshot_by_root(&root_1).is_some());
        assert!(restored.snapshot_by_root(&root_0).is_some());
    }

    #[test]
    fn concurrent_reads_dont_panic() {
        use std::sync::Arc;
        let store = Arc::new(TrustStore::new());
        let (root, _) = make_root(0x01);
        let (peer, _) = store.resolve_or_observe(root);

        let handles: Vec<_> = (0..8).map(|_| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    let _ = store.state(&peer);
                    let _ = store.snapshot(&peer);
                    let _ = store.is_within_grace(&peer, &Hlc::now());
                    let _ = store.resolve_or_observe(root);
                }
            })
        }).collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn concurrent_rotation_and_resolve_no_split_identity() {
        use std::sync::Arc;
        let store = Arc::new(TrustStore::new());
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (anchor, _) = store.resolve_or_observe(root_0);

        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();

        let store_writer = Arc::clone(&store);
        let writer = std::thread::spawn(move || {
            store_writer.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();
        });

        let handles: Vec<_> = (0..4).map(|_| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                for _ in 0..100 {
                    let _ = store.resolve_or_observe(root_1);
                }
            })
        }).collect();

        writer.join().unwrap();
        for h in handles {
            h.join().unwrap();
        }

        let (peer, _) = store.resolve_or_observe(root_1);
        assert_eq!(*peer.root(), root_0,
            "root_1 must resolve to root_0 anchor");
        assert_eq!(store.len(), 1,
            "exactly 1 peer — no split identity");
    }

    #[test]
    fn apply_rotation_same_shard_no_deadlock() {
        // WS-3.3: Force anchor_root and new_head to the same DashMap shard.
        // The phased acquisition (drop get_mut guard before remove) prevents
        // the self-deadlock. This test proves it survives the colliding case.
        //
        // Strategy: use DashMap::hasher() (public) to compute hashes for
        // candidate roots. Two keys collide on shard when their hashes agree
        // modulo the shard count. DashMap v6 uses (hash >> 7) % shard_count
        // internally, but since we can't access shard_count directly, we
        // try all plausible power-of-2 shard counts (4..=128) and accept a
        // candidate that collides under ANY of them — guaranteeing at least
        // one real collision.
        use std::hash::{BuildHasher, Hash, Hasher};

        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (anchor, _) = store.resolve_or_observe(root_0);

        let build_hasher = store.records.hasher().clone();

        fn hash_key(bh: &impl BuildHasher, key: &IdentityRoot) -> u64 {
            let mut h = bh.build_hasher();
            key.hash(&mut h);
            h.finish()
        }

        let anchor_hash = hash_key(&build_hasher, &root_0);

        // Find a root whose hash collides with anchor under some shard count.
        // With 254 candidates and shard counts 4..=128, we're testing
        // hash % 4, hash % 8, ... hash % 128. The birthday bound makes
        // collision near-certain.
        let mut colliding_root = None;
        for byte in 2..=255u8 {
            let (candidate, _, ) = make_root(byte);
            let candidate_hash = hash_key(&build_hasher, &candidate);
            // Check collision under several plausible shard counts
            for shift in [0u32, 7] {
                let a = (anchor_hash >> shift) as usize;
                let c = (candidate_hash >> shift) as usize;
                for shard_count in [4, 8, 16, 32, 64, 128] {
                    if a % shard_count == c % shard_count {
                        colliding_root = Some(candidate);
                        break;
                    }
                }
                if colliding_root.is_some() { break; }
            }
            if colliding_root.is_some() { break; }
        }

        let new_head = colliding_root
            .expect("could not find a shard-colliding root in 254 candidates");

        // Create a provisional at the colliding root
        store.resolve_or_observe(new_head);
        assert_eq!(store.len(), 2, "anchor + provisional");

        // Build rotation chain from root_0 → new_head
        let proof = RotationProof::create(
            &seed_0, root_0, new_head, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();

        // Run with a timeout — if the phased acquisition is broken, this hangs.
        let store_arc = std::sync::Arc::new(store);
        let store_clone = std::sync::Arc::clone(&store_arc);
        let handle = std::thread::spawn(move || {
            store_clone.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();
        });

        // Join with timeout — 5 seconds is generous for a non-deadlocking operation.
        let result = handle.join();
        assert!(result.is_ok(),
            "apply_rotation with same-shard anchor+new_head must not deadlock");

        // Verify the merge happened correctly
        assert_eq!(store_arc.len(), 1,
            "provisional must be merged, not left as separate entry");
    }

    #[test]
    fn dead_root_blocks_re_observation() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        store.resolve_or_observe(root);

        let death = DeathNotice {
            root,
            epoch: RotationEpoch::ORIGIN,
            issued_at: Hlc::now(),
            signature: crate::wire::signable::Signature64::ZERO,
        };
        let verified_death = Verified::new_for_test(death);
        store.apply_death(&verified_death);
        assert_eq!(store.len(), 0);

        // Re-observe the dead root — must NOT create a new record
        store.resolve_or_observe(root);
        assert_eq!(store.len(), 0,
            "dead root must not be resurrected by re-observation");
    }

    #[test]
    fn import_rebuilds_prior_head() {
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, seed_1) = make_root(0x02);
        let (root_2, _) = make_root(0x03);
        let (anchor, _) = store.resolve_or_observe(root_0);

        // Two rotations: root_0 → root_1 → root_2
        let p1 = RotationProof::create(&seed_0, root_0, root_1, RotationEpoch(1), Hlc::now()).unwrap();
        let c1 = RotationChain::verify((root_0, RotationEpoch::ORIGIN), vec![p1]).unwrap();
        store.apply_rotation(&anchor, &c1, Hlc::now()).unwrap();

        let p2 = RotationProof::create(&seed_1, root_1, root_2, RotationEpoch(2), Hlc::now()).unwrap();
        let c2 = RotationChain::verify((root_1, RotationEpoch(1)), vec![p2]).unwrap();
        store.apply_rotation(&anchor, &c2, Hlc::now()).unwrap();

        // Export and import
        let exported = store.export();
        let restored = TrustStore::import(exported);

        // Current head resolves
        assert!(restored.snapshot_by_root(&root_2).is_some(), "current head must resolve after import");
        // Prior head resolves (rebuilt from prior_head_root)
        assert!(restored.snapshot_by_root(&root_1).is_some(), "prior head must resolve after import");
        // Anchor resolves via primary key
        assert!(restored.snapshot_by_root(&root_0).is_some(), "anchor must resolve after import");

        // A third rotation should correctly evict root_1 from the index
        let (root_3, _) = make_root(0x04);
        let seed_2 = make_root(0x03).1;
        let p3 = RotationProof::create(&seed_2, root_2, root_3, RotationEpoch(3), Hlc::now()).unwrap();
        let c3 = RotationChain::verify((root_2, RotationEpoch(2)), vec![p3]).unwrap();
        let (anchor_restored, _) = restored.resolve_or_observe(root_0);
        restored.apply_rotation(&anchor_restored, &c3, Hlc::now()).unwrap();

        assert!(restored.head_to_anchor_get(&root_1).is_none(),
            "prior-prior head must be evicted after rotation post-import");
    }

    #[test]
    fn pin_current_root_resolves_pin_violation() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        let (new_root, _) = make_root(0x02);
        let (peer, _) = store.resolve_or_observe(root);

        store.unexplained_root_change(&peer, new_root).unwrap();
        let record = store.snapshot(&peer).unwrap();
        assert_eq!(record.state, TrustState::PinViolation);

        let state = store.pin_current_root(&peer).unwrap();
        assert_eq!(state, TrustState::Pinned);

        let record = store.snapshot(&peer).unwrap();
        assert_eq!(record.pinned_root, record.anchor_root,
            "pin_current_root must update pinned_root to anchor_root");
    }

    #[test]
    fn withdraw_verification_clears_latch_and_pins() {
        let store = TrustStore::new();
        let (root, _) = make_root(0x01);
        let (new_root, _) = make_root(0x02);
        let (peer, _) = store.resolve_or_observe(root);

        store.mark_verified(&peer).unwrap();
        store.unexplained_root_change(&peer, new_root).unwrap();
        let record = store.snapshot(&peer).unwrap();
        assert_eq!(record.state, TrustState::VerificationViolation);
        assert!(record.previously_verified);

        let state = store.withdraw_verification(&peer).unwrap();
        assert_eq!(state, TrustState::Pinned);

        let record = store.snapshot(&peer).unwrap();
        assert!(!record.previously_verified,
            "withdraw_verification must clear the latch");
        assert_eq!(record.pinned_root, record.anchor_root,
            "withdraw_verification must pin current root");

        // Future unexplained change should be PinViolation, not VerificationViolation
        let (another_root, _) = make_root(0x03);
        let state = store.unexplained_root_change(&peer, another_root).unwrap();
        assert_eq!(state, TrustState::PinViolation,
            "after withdrawal, violations are pin-level, not verification-level");
    }

    #[test]
    fn cascade_own_trust_sets_latch_on_signed_peers() {
        let store = TrustStore::new();
        let (root_a, _) = make_root(0x01);
        let (root_b, _) = make_root(0x02);
        let (root_c, _) = make_root(0x03);

        store.resolve_or_observe(root_a);
        store.resolve_or_observe(root_b);
        store.resolve_or_observe(root_c);

        // Simulate: root_a and root_b are signed by us, root_c is not
        let signed_roots: std::collections::HashSet<IdentityRoot> =
            [root_a, root_b].into_iter().collect();

        let updated = store.cascade_own_trust_change(|root| signed_roots.contains(root));
        assert_eq!(updated, 2);

        let record_a = store.snapshot_by_root(&root_a).unwrap();
        assert!(record_a.previously_verified);
        let record_b = store.snapshot_by_root(&root_b).unwrap();
        assert!(record_b.previously_verified);
        let record_c = store.snapshot_by_root(&root_c).unwrap();
        assert!(!record_c.previously_verified, "unsigned peer must not be affected");
    }

    #[test]
    fn event_emission_significant_changes_only() {
        let store = TrustStore::new();
        let rx = store.subscribe();

        let (root, _) = make_root(0x01);
        let (new_root, _) = make_root(0x02);
        let (peer, _) = store.resolve_or_observe(root);

        // Pinned → Verified: insignificant (no warning needed)
        store.mark_verified(&peer).unwrap();
        assert!(rx.try_recv().is_err(), "Pinned→Verified should not emit");

        // Verified → VerificationViolation: significant (identity changed after verify)
        store.unexplained_root_change(&peer, new_root).unwrap();
        let event = rx.try_recv().expect("Verified→VerificationViolation must emit");
        assert_eq!(event.old_state, TrustState::Verified);
        assert_eq!(event.new_state, TrustState::VerificationViolation);
        assert!(event.is_significant());

        // VerificationViolation → Pinned via withdraw: significant (resolution)
        store.withdraw_verification(&peer).unwrap();
        let event = rx.try_recv().expect("VerificationViolation→Pinned must emit");
        assert_eq!(event.new_state, TrustState::Pinned);
    }
}
