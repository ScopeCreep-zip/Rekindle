//! TrustStore — sharded in-memory registry of per-peer trust records.
//!
//! Three internal DashMaps keyed by `IdentityRoot`:
//! - `records`: anchor root → TrustRecord (primary, key never changes)
//! - `head_to_anchor`: current-head root → anchor root (secondary index)
//! - `prior_head`: anchor root → prior head root (eviction tracking)
//!
//! All mutation paths acquire `invariant_lock.write()`. Index-touching
//! read paths (`snapshot_by_root`) acquire `invariant_lock.read()`.
//! Pure primary-key reads (`state`, `snapshot`, `is_within_grace`) are
//! lock-free — they take `&PeerRef` and go straight to `records` via
//! `peer.root()`, never touching the index.
//!
//! `resolve_or_observe` is the single external entry point for turning
//! an observed root into a `PeerRef`. It acquires the write lock because
//! its Arm 3 (first contact) mutates.
//!
//! **Provisional identity reconciliation:** when `apply_rotation`
//! advances a peer to `new_head`, it checks whether `new_head` already
//! exists as a primary key (a provisional first-contact record created
//! before the rotation proof arrived — the common case in a distributed
//! system where you encounter a peer's new head via a message before
//! their rotation proof propagates). If so, `apply_rotation` merges the
//! provisional record into the anchor's record and removes the orphan.
//! This prevents the split-identity defect where one peer has two
//! primary entries. The merge rule: `Verified` state and the
//! `previously_verified` latch transfer (positive assertions survive);
//! violation states on the provisional do not transfer (they were
//! triggered by a view that the merge corrects).

use std::sync::RwLock;

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

pub struct TrustStore {
    records: DashMap<IdentityRoot, TrustRecord>,
    head_to_anchor: DashMap<IdentityRoot, IdentityRoot>,
    prior_head: DashMap<IdentityRoot, IdentityRoot>,
    /// Write side: all mutation paths.
    /// Read side: `snapshot_by_root` (index-touching read).
    /// Lock-free: `state`, `snapshot`, `is_within_grace`.
    invariant_lock: RwLock<()>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self {
            records: DashMap::new(),
            head_to_anchor: DashMap::new(),
            prior_head: DashMap::new(),
            invariant_lock: RwLock::new(()),
        }
    }

    // ── The single external door ────────────────────────────────

    /// Given an observed root, returns the stable `PeerRef` and current
    /// trust state. Handles known-head, known-anchor, and first-contact
    /// atomically under the write lock.
    pub fn resolve_or_observe(&self, root: IdentityRoot) -> (PeerRef, TrustState) {
        eprintln!("[resolve_or_observe] acquiring invariant_lock.write for {:?}", &root.as_bytes()[..4]);
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        eprintln!("[resolve_or_observe] lock acquired");

        if let Some(anchor_root) = self.head_to_anchor_get(&root) {
            eprintln!("[resolve_or_observe] Arm 1: head found in index");
            let state = self.records.get(&anchor_root)
                .map(|r| r.state)
                .unwrap_or(TrustState::Pinned);
            return (PeerRef::anchor(anchor_root), state);
        }
        if let Some(entry) = self.records.get(&root) {
            eprintln!("[resolve_or_observe] Arm 2: root is primary key");
            return (PeerRef::anchor(root), entry.state);
        }
        eprintln!("[resolve_or_observe] Arm 3: first contact");
        self.observe_root_internal(root)
    }

    // ── Internal helpers ────────────────────────────────────────

    fn head_to_anchor_get(&self, root: &IdentityRoot) -> Option<IdentityRoot> {
        self.head_to_anchor.get(root).map(|e| *e.value())
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
        eprintln!("[observe_root_internal] acquiring records.entry");
        let entry = self.records.entry(root);
        match entry {
            dashmap::mapref::entry::Entry::Vacant(v) => {
                eprintln!("[observe_root_internal] vacant — inserting first contact");
                let record = TrustRecord::first_contact(root);
                let state = record.state;
                v.insert(record);
                (PeerRef::anchor(root), state)
            }
            dashmap::mapref::entry::Entry::Occupied(o) => {
                eprintln!("[observe_root_internal] occupied — returning existing");
                (PeerRef::anchor(root), o.get().state)
            }
        }
    }

    // ── Rotation with provisional reconciliation ────────────────

    /// Apply a verified rotation chain. Advances `pinned_root`, opens
    /// the grace window, updates the secondary index.
    ///
    /// **Provisional reconciliation:** if `new_head` already exists as
    /// a primary key (a first-contact record created before the rotation
    /// proof arrived), the provisional record is merged into the anchor
    /// and the orphan is removed.
    ///
    /// Merge rule (single rule, all paths through `transition()`):
    /// - `previously_verified` latch: always OR'd into anchor (latches never clear).
    /// - `Verified` on provisional: `transition(VerificationSucceeded)` → `Verified`.
    /// - `VerificationViolation` or `PinViolation` on provisional:
    ///   `transition(UnexplainedRootChange)` with the merged latch. The table
    ///   produces `VerificationViolation` if latch is set, `PinViolation` if not.
    ///   The rotation proof does NOT cover the hop the violation recorded.
    /// - `Pinned` on provisional: no state change on anchor (nothing to transfer).
    ///
    /// No direct state pokes — every assignment flows through `transition()`.
    pub fn apply_rotation(
        &self,
        peer: &PeerRef,
        chain: &RotationChain,
        now: Hlc,
    ) -> Result<TrustState, IdentityError> {
        eprintln!("[apply_rotation] acquiring invariant_lock.write");
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        let anchor_root = *peer.root();

        // Phase 1: read what we need from the anchor entry, then drop the guard.
        // This prevents the DashMap shard deadlock where get_mut(anchor) and
        // remove(new_head) hash to the same shard.
        eprintln!("[apply_rotation] acquiring records.get_mut(anchor) — phase 1 read");
        let (old_head, new_head, anchor_state, anchor_latch) = {
            let mut entry = self.records.get_mut(&anchor_root)
                .ok_or(IdentityError::Encoding("peer not in trust store".into()))?;
            eprintln!("[apply_rotation] got records entry");

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

            (old_head, new_head, new_state, new_latch)
        }; // ← guard dropped here, shard lock released
        eprintln!("[apply_rotation] phase 1 done, anchor guard dropped");

        // Phase 2: provisional reconciliation (no anchor guard held).
        // final_state starts as Phase 1's result, overwritten if merge happens.
        let mut final_state = anchor_state;
        if new_head != anchor_root {
            eprintln!("[apply_rotation] checking for provisional at new_head");
            if let Some((_, provisional)) = self.records.remove(&new_head) {
                eprintln!("[apply_rotation] provisional removed, merging state={:?} latch={}",
                    provisional.state, provisional.previously_verified);

                // Compute merged state through the transition table.
                // The provisional's state represents an observation that
                // may or may not survive the merge.
                let mut merged_state = anchor_state;
                let mut merged_latch = anchor_latch;

                // Latch always OR'd — latches never clear.
                if provisional.previously_verified {
                    merged_latch = true;
                }

                // State merge: the provisional's observation persists unless
                // it's just Pinned (nothing to transfer).
                match provisional.state {
                    TrustState::Verified => {
                        // Positive assertion: verification survives.
                        let (s, l) = transition(merged_state, merged_latch,
                            TrustEvent::VerificationSucceeded);
                        merged_state = s;
                        merged_latch = l;
                    }
                    TrustState::VerificationViolation | TrustState::PinViolation => {
                        // The provisional saw an unexplained change that the
                        // rotation proof does NOT cover. Surface it as a
                        // root change on the anchor — the transition table
                        // will produce VerificationViolation (if latch set)
                        // or PinViolation (if not), which is correct.
                        let (s, l) = transition(merged_state, merged_latch,
                            TrustEvent::UnexplainedRootChange);
                        merged_state = s;
                        merged_latch = l;
                    }
                    TrustState::Pinned => {
                        // Nothing to transfer.
                    }
                }

                // Phase 3: re-acquire anchor entry to write merged state.
                // Under invariant_lock.write(), nothing can remove the anchor
                // between Phase 1 drop and this re-acquire. If it's gone,
                // the invariant is violated — panic, don't skip.
                eprintln!("[apply_rotation] re-acquiring anchor entry for merge write");
                let mut entry = self.records.get_mut(&anchor_root)
                    .expect("anchor entry vanished under write lock — invariant violation");
                entry.state = merged_state;
                entry.previously_verified = merged_latch;
                eprintln!("[apply_rotation] merge written: state={:?} latch={}",
                    merged_state, merged_latch);
                drop(entry);

                // Clean provisional's index entries
                self.head_to_anchor.retain(|_, anchor| *anchor != new_head);
                self.prior_head.remove(&new_head);

                // Return merged state directly — no re-read from map.
                final_state = merged_state;
            } else {
                eprintln!("[apply_rotation] no provisional found");
            }
        }

        // Phase 4: index maintenance (no records guard held).
        if let Some((_, prior_prior)) = self.prior_head.remove(&anchor_root) {
            if prior_prior != anchor_root {
                self.head_to_anchor.remove(&prior_prior);
            }
        }
        if old_head != anchor_root {
            self.prior_head.insert(anchor_root, old_head);
        }
        self.head_to_anchor.insert(new_head, anchor_root);
        eprintln!("[apply_rotation] complete, returning state={:?}", final_state);

        Ok(final_state)
    }

    // ── Revocation ──────────────────────────────────────────────

    pub fn apply_revocation(
        &self,
        cert: &Verified<RevocationCertificate>,
    ) -> Result<TrustState, IdentityError> {
        eprintln!("[apply_revocation] acquiring invariant_lock.write");
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        eprintln!("[apply_revocation] lock acquired");
        let cert = cert.get();

        let anchor_root = self.resolve_anchor_root(&cert.root)
            .ok_or(IdentityError::Encoding("revocation: peer not in trust store".into()))?;

        let mut entry = self.records.get_mut(&anchor_root)
            .ok_or(IdentityError::Encoding("revocation: record disappeared".into()))?;

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
        eprintln!("[apply_death] acquiring invariant_lock.write");
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        eprintln!("[apply_death] lock acquired");
        let notice = notice.get();

        let anchor_root = self.resolve_anchor_root(&notice.root)?;

        let removed = self.records.remove(&anchor_root).map(|(_, record)| record);
        self.head_to_anchor.retain(|_, anchor| *anchor != anchor_root);
        self.prior_head.remove(&anchor_root);

        removed
    }

    // ── Verification / acknowledgement ──────────────────────────

    pub fn mark_verified(
        &self,
        peer: &PeerRef,
    ) -> Result<TrustState, IdentityError> {
        eprintln!("[mark_verified] acquiring invariant_lock.write");
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        eprintln!("[mark_verified] lock acquired");

        let mut entry = self.records.get_mut(peer.root())
            .ok_or(IdentityError::Encoding("peer not in trust store".into()))?;

        let (new_state, new_latch) = transition(
            entry.state,
            entry.previously_verified,
            TrustEvent::VerificationSucceeded,
        );
        entry.state = new_state;
        entry.previously_verified = new_latch;

        Ok(new_state)
    }

    pub fn acknowledge(
        &self,
        peer: &PeerRef,
    ) -> Result<TrustState, IdentityError> {
        eprintln!("[acknowledge] acquiring invariant_lock.write");
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        eprintln!("[acknowledge] lock acquired");

        let mut entry = self.records.get_mut(peer.root())
            .ok_or(IdentityError::Encoding("peer not in trust store".into()))?;

        let (new_state, new_latch) = transition(
            entry.state,
            entry.previously_verified,
            TrustEvent::UserAcknowledged,
        );
        entry.state = new_state;
        entry.previously_verified = new_latch;

        Ok(new_state)
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
        eprintln!("[unexplained_root_change] acquiring invariant_lock.write");
        let _guard = self.invariant_lock.write().expect("lock poisoned");
        eprintln!("[unexplained_root_change] lock acquired");

        let mut entry = self.records.get_mut(peer.root())
            .ok_or(IdentityError::Encoding("peer not in trust store".into()))?;

        let (new_state, new_latch) = transition(
            entry.state,
            entry.previously_verified,
            TrustEvent::UnexplainedRootChange,
        );
        entry.state = new_state;
        entry.previously_verified = new_latch;
        entry.pinned_root = new_root;
        entry.void_grace();

        Ok(new_state)
    }

    // ── Read operations ─────────────────────────────────────────

    /// Lock-free: goes straight to primary map via anchor root.
    pub fn snapshot(&self, peer: &PeerRef) -> Option<TrustRecord> {
        self.records.get(peer.root()).map(|entry| entry.clone())
    }

    /// Takes the read lock: touches the index to resolve head → anchor.
    pub fn snapshot_by_root(&self, root: &IdentityRoot) -> Option<TrustRecord> {
        eprintln!("[snapshot_by_root] acquiring invariant_lock.read");
        let _guard = self.invariant_lock.read().expect("lock poisoned");
        eprintln!("[snapshot_by_root] read lock acquired");
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
            if record.pinned_root != anchor {
                store.head_to_anchor.insert(record.pinned_root, anchor);
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

        // First contact with root_0 (the real anchor)
        let (anchor, _) = store.resolve_or_observe(root_0);

        // root_1 observed as first contact BEFORE the rotation proof arrives.
        // This is the common case: you see a peer's new head in a message
        // before their rotation proof propagates through the DHT.
        let (provisional_peer, _) = store.resolve_or_observe(root_1);
        assert_ne!(anchor, provisional_peer, "before merge: two separate peers");
        assert_eq!(store.len(), 2, "before merge: two primary entries");

        // Verify the provisional identity independently
        store.mark_verified(&provisional_peer).unwrap();
        let provisional_record = store.snapshot(&provisional_peer).unwrap();
        assert!(provisional_record.previously_verified);

        // Now the rotation proof arrives
        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        store.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();

        // After merge: exactly 1 primary entry
        assert_eq!(store.len(), 1, "after merge: provisional orphan removed");

        // root_1 resolves to the anchor
        let (resolved, _) = store.resolve_or_observe(root_1);
        assert_eq!(resolved, anchor, "root_1 must resolve to root_0 anchor");

        // The provisional's Verified state and latch transferred
        let merged_record = store.snapshot(&anchor).unwrap();
        assert!(merged_record.previously_verified,
            "provisional verification must survive merge");
        // pinned_root is root_1 (the proof's head), not whatever the
        // provisional may have advanced to. The merge removes the
        // provisional record; the anchor's pinned_root was set by
        // Phase 1's advance_rotation.
        assert_eq!(merged_record.pinned_root, root_1,
            "pinned_root must be the rotation proof's head");
    }

    #[test]
    fn merge_provisional_verification_violation_transfers_as_pin_violation() {
        // A provisional identity was verified, then saw an unexplained
        // change (VerificationViolation). The rotation proof arrives,
        // merging the provisional into the anchor. The violation signal
        // must NOT be silently discarded — it transfers as PinViolation
        // on the anchor with the latch preserved, because the human
        // saw something change and that signal must survive.
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (root_change, _) = make_root(0x03);

        let (anchor, _) = store.resolve_or_observe(root_0);
        let (provisional, _) = store.resolve_or_observe(root_1);

        // Verify the provisional, then trigger VerificationViolation
        store.mark_verified(&provisional).unwrap();
        store.unexplained_root_change(&provisional, root_change).unwrap();

        let prov_record = store.snapshot(&provisional).unwrap();
        assert_eq!(prov_record.state, TrustState::VerificationViolation);
        assert!(prov_record.previously_verified);

        // Rotation proof arrives linking root_0 → root_1
        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        store.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();

        assert_eq!(store.len(), 1, "provisional merged");

        let merged = store.snapshot(&anchor).unwrap();
        // The latch MUST survive — the human verified something.
        assert!(merged.previously_verified,
            "latch must survive merge from VerificationViolation provisional");
        // The violation transfers as VerificationViolation — the latch is
        // set (OR'd from the provisional), and the transition table says
        // latch + unexplained change = VerificationViolation. The rotation
        // proof explains the identity topology (root_0 = root_1) but NOT
        // the change the human observed (root_1 → root_change). The table
        // is the authority; no override.
        assert_eq!(merged.state, TrustState::VerificationViolation,
            "latch=true + unexplained hop = VerificationViolation per transition table");
        // pinned_root is root_1 (the proof's head). root_change (the
        // provisional's advanced head from the unexplained hop) is lost —
        // the merge removes the provisional, and the proof only covers
        // root_0 → root_1. This is documented and intentional: the
        // unexplained hop is signaled through the state (PinViolation)
        // but the root itself is not carried forward.
        assert_eq!(merged.pinned_root, root_1,
            "pinned_root must be the rotation proof's head, not root_change");
    }

    #[test]
    fn merge_provisional_pin_violation_transfers() {
        // A provisional identity had an unexplained change (PinViolation)
        // before the rotation proof arrived. The unexplained hop
        // (root_1 → root_change) is NOT covered by the rotation proof
        // (which covers root_0 → root_1). The violation must survive
        // the merge as PinViolation on the anchor.
        let store = TrustStore::new();
        let (root_0, seed_0) = make_root(0x01);
        let (root_1, _) = make_root(0x02);
        let (root_change, _) = make_root(0x03);

        let (anchor, _) = store.resolve_or_observe(root_0);
        let (provisional, _) = store.resolve_or_observe(root_1);

        // Trigger PinViolation on the provisional (no prior verification)
        store.unexplained_root_change(&provisional, root_change).unwrap();
        let prov_record = store.snapshot(&provisional).unwrap();
        assert_eq!(prov_record.state, TrustState::PinViolation);
        assert!(!prov_record.previously_verified);

        // Rotation proof arrives
        let proof = RotationProof::create(
            &seed_0, root_0, root_1, RotationEpoch(1), Hlc::now(),
        ).unwrap();
        let chain = RotationChain::verify(
            (root_0, RotationEpoch::ORIGIN), vec![proof],
        ).unwrap();
        store.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();

        assert_eq!(store.len(), 1, "provisional merged");

        let merged = store.snapshot(&anchor).unwrap();
        // The unexplained hop survives — the rotation proof doesn't cover it.
        // Latch was never set, so this is PinViolation (not VerificationViolation).
        assert!(!merged.previously_verified);
        assert_eq!(merged.state, TrustState::PinViolation,
            "PinViolation on provisional must transfer — the unexplained hop is still unexplained");
        // pinned_root is root_1 (proof's head). root_change is lost.
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
        let verified_cert = Verified::new_trusted(cert);
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
        let verified_death = Verified::new_trusted(death);
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
        let verified_death = Verified::new_trusted(death);
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
            eprintln!("[writer] acquiring lock for apply_rotation");
            store_writer.apply_rotation(&anchor, &chain, Hlc::now()).unwrap();
            eprintln!("[writer] apply_rotation complete");
        });

        let handles: Vec<_> = (0..4).map(|i| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                eprintln!("[resolver-{i}] starting 100 iterations");
                for j in 0..100 {
                    eprintln!("[resolver-{i}] iteration {j} acquiring lock");
                    let _ = store.resolve_or_observe(root_1);
                    eprintln!("[resolver-{i}] iteration {j} done");
                }
                eprintln!("[resolver-{i}] complete");
            })
        }).collect();

        eprintln!("[main] joining writer");
        writer.join().unwrap();
        eprintln!("[main] writer joined, joining resolvers");
        for (i, h) in handles.into_iter().enumerate() {
            eprintln!("[main] joining resolver-{i}");
            h.join().unwrap();
            eprintln!("[main] resolver-{i} joined");
        }
        eprintln!("[main] all joined");

        let (peer, _) = store.resolve_or_observe(root_1);
        assert_eq!(*peer.root(), root_0,
            "root_1 must resolve to root_0 anchor");
        assert_eq!(store.len(), 1,
            "exactly 1 peer — no split identity");
    }
}
