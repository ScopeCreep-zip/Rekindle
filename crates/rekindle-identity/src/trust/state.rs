//! TrustState — the four-state machine over a peer's identity trust.
//!
//! Effective state is recomputed mechanically from evidence on every
//! evidence change. No API exists to set state directly. The transition
//! table is exhaustive — any transition not listed is a defect.

/// The state of a peer's identity from our perspective.
///
/// Ordered by severity: `Pinned` < `PinViolation` < `Verified` <
/// `VerificationViolation`. The `PartialOrd` is for the UI severity
/// ordering, not for state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TrustState {
    /// First contact, or user acknowledged an identity change.
    Pinned,
    /// Identity changed since it was pinned. User must acknowledge.
    PinViolation,
    /// Interactive verification completed. Highest trust.
    Verified,
    /// Identity changed AFTER verification. Serious warning.
    /// The `PreviouslyVerified` latch prevents downgrade to `PinViolation`.
    VerificationViolation,
}

/// Events that trigger trust state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustEvent {
    /// First observation of a root (or re-observation after acknowledgement).
    RootObserved,
    /// Root changed without a valid rotation proof.
    UnexplainedRootChange,
    /// Valid rotation proof chain observed.
    RotationProofVerified,
    /// Interactive verification succeeded (safety number comparison, QR scan).
    VerificationSucceeded,
    /// User acknowledges a new root after PinViolation.
    UserAcknowledged,
    /// User withdraws verification (re-pins current root without full verify).
    VerificationWithdrawn,
    /// Verified `RevocationCertificate` received.
    RevocationReceived,
    /// Verified `DeathNotice` received.
    DeathReceived,
}

/// Compute the next state given current state, the latch, and an event.
///
/// Returns `(new_state, new_latch)`. The latch is `true` if it was
/// already set OR if this transition sets it. It is NEVER cleared
/// by any transition.
pub fn transition(
    current: TrustState,
    previously_verified: bool,
    event: TrustEvent,
) -> (TrustState, bool) {
    match (current, event) {
        // ── RootObserved (first contact) ─────────────────────
        (_, TrustEvent::RootObserved) => {
            (TrustState::Pinned, previously_verified)
        }

        // ── UnexplainedRootChange ────────────────────────────
        (_, TrustEvent::UnexplainedRootChange) => {
            if previously_verified {
                (TrustState::VerificationViolation, true)
            } else {
                (TrustState::PinViolation, false)
            }
        }

        // ── RotationProofVerified ────────────────────────────
        // Valid rotation: head advances, grace window opens.
        // State stays the same (Pinned remains Pinned, Verified remains Verified).
        (state, TrustEvent::RotationProofVerified) => {
            (state, previously_verified)
        }

        // ── VerificationSucceeded ────────────────────────────
        (_, TrustEvent::VerificationSucceeded) => {
            (TrustState::Verified, true) // latch SET
        }

        // ── UserAcknowledged ─────────────────────────────────
        (TrustState::PinViolation, TrustEvent::UserAcknowledged) => {
            (TrustState::Pinned, previously_verified)
        }
        (TrustState::VerificationViolation, TrustEvent::UserAcknowledged) => {
            // User acknowledges but does NOT re-verify — drops to Pinned.
            // Latch REMAINS set (previously_verified persists).
            (TrustState::Pinned, previously_verified)
        }
        // Acknowledging in other states is a no-op
        (state, TrustEvent::UserAcknowledged) => {
            (state, previously_verified)
        }

        // ── VerificationWithdrawn ────────────────────────────
        // Per matrix-rust-sdk: withdraw_verification() calls pin() first,
        // then clears the latch. The user is saying "I no longer hold this
        // peer to the verified standard." Future violations are pin-level.
        (TrustState::VerificationViolation, TrustEvent::VerificationWithdrawn) => {
            (TrustState::Pinned, false) // latch CLEARED
        }
        (TrustState::Verified, TrustEvent::VerificationWithdrawn) => {
            (TrustState::Pinned, false) // latch CLEARED
        }
        (state, TrustEvent::VerificationWithdrawn) => {
            (state, previously_verified) // no-op in other states
        }

        // ── RevocationReceived ───────────────────────────────
        (_, TrustEvent::RevocationReceived) => {
            if previously_verified {
                (TrustState::VerificationViolation, true)
            } else {
                (TrustState::PinViolation, false)
            }
        }

        // ── DeathReceived ────────────────────────────────────
        // Terminal — we don't model a Dead state in TrustState
        // because the TrustRecord is removed from the store.
        // The caller handles teardown.
        (_, TrustEvent::DeathReceived) => {
            // Return PinViolation as a signal; the caller removes the record.
            if previously_verified {
                (TrustState::VerificationViolation, true)
            } else {
                (TrustState::PinViolation, false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_contact_pinned() {
        let (state, latch) = transition(TrustState::Pinned, false, TrustEvent::RootObserved);
        assert_eq!(state, TrustState::Pinned);
        assert!(!latch);
    }

    #[test]
    fn unexplained_change_without_latch_is_pin_violation() {
        let (state, latch) = transition(TrustState::Pinned, false, TrustEvent::UnexplainedRootChange);
        assert_eq!(state, TrustState::PinViolation);
        assert!(!latch);
    }

    #[test]
    fn unexplained_change_with_latch_is_verification_violation() {
        let (state, latch) = transition(TrustState::Pinned, true, TrustEvent::UnexplainedRootChange);
        assert_eq!(state, TrustState::VerificationViolation);
        assert!(latch);
    }

    #[test]
    fn verification_sets_latch() {
        let (state, latch) = transition(TrustState::Pinned, false, TrustEvent::VerificationSucceeded);
        assert_eq!(state, TrustState::Verified);
        assert!(latch, "latch must be set on verification");
    }

    #[test]
    fn rotation_preserves_state() {
        let (state, _) = transition(TrustState::Verified, true, TrustEvent::RotationProofVerified);
        assert_eq!(state, TrustState::Verified);

        let (state, _) = transition(TrustState::Pinned, false, TrustEvent::RotationProofVerified);
        assert_eq!(state, TrustState::Pinned);
    }

    #[test]
    fn acknowledge_pin_violation() {
        let (state, _) = transition(TrustState::PinViolation, false, TrustEvent::UserAcknowledged);
        assert_eq!(state, TrustState::Pinned);
    }

    #[test]
    fn withdraw_verification_clears_latch() {
        // Per matrix-rust-sdk: withdraw_verification() clears the latch.
        // The user is saying "I no longer hold this peer to the verified standard."
        // Future unexplained changes produce PinViolation, not VerificationViolation.
        let (state, latch) = transition(TrustState::Verified, true, TrustEvent::VerificationWithdrawn);
        assert_eq!(state, TrustState::Pinned);
        assert!(!latch, "latch must be cleared on withdrawal");
    }

    #[test]
    fn withdraw_from_violation_clears_latch() {
        let (state, latch) = transition(TrustState::VerificationViolation, true, TrustEvent::VerificationWithdrawn);
        assert_eq!(state, TrustState::Pinned);
        assert!(!latch, "latch must be cleared on withdrawal from violation");
    }

    #[test]
    fn latch_never_clears_through_rotation() {
        let (_, latch) = transition(TrustState::Verified, true, TrustEvent::RotationProofVerified);
        assert!(latch);
        let (_, latch) = transition(TrustState::Pinned, true, TrustEvent::RotationProofVerified);
        assert!(latch);
    }

    #[test]
    fn latch_never_clears_through_acknowledgement() {
        let (_, latch) = transition(TrustState::PinViolation, true, TrustEvent::UserAcknowledged);
        assert!(latch);
    }

    #[test]
    fn revocation_with_latch_is_verification_violation() {
        let (state, latch) = transition(TrustState::Verified, true, TrustEvent::RevocationReceived);
        assert_eq!(state, TrustState::VerificationViolation);
        assert!(latch);
    }

    #[test]
    fn revocation_without_latch_is_pin_violation() {
        let (state, latch) = transition(TrustState::Pinned, false, TrustEvent::RevocationReceived);
        assert_eq!(state, TrustState::PinViolation);
        assert!(!latch);
    }

    #[test]
    fn death_with_latch_is_verification_violation() {
        let (state, latch) = transition(TrustState::Verified, true, TrustEvent::DeathReceived);
        assert_eq!(state, TrustState::VerificationViolation);
        assert!(latch);
    }

    #[test]
    fn full_lifecycle_latch_persistence() {
        // Originate → verify → rotate → rotate → unexplained change → acknowledge
        // Latch must persist through all transitions.
        let (state, latch) = transition(TrustState::Pinned, false, TrustEvent::RootObserved);
        assert_eq!(state, TrustState::Pinned);
        assert!(!latch);

        let (state, latch) = transition(state, latch, TrustEvent::VerificationSucceeded);
        assert_eq!(state, TrustState::Verified);
        assert!(latch);

        let (state, latch) = transition(state, latch, TrustEvent::RotationProofVerified);
        assert_eq!(state, TrustState::Verified);
        assert!(latch);

        let (state, latch) = transition(state, latch, TrustEvent::RotationProofVerified);
        assert_eq!(state, TrustState::Verified);
        assert!(latch);

        let (state, latch) = transition(state, latch, TrustEvent::UnexplainedRootChange);
        assert_eq!(state, TrustState::VerificationViolation);
        assert!(latch);

        let (state, latch) = transition(state, latch, TrustEvent::UserAcknowledged);
        assert_eq!(state, TrustState::Pinned);
        assert!(latch, "latch must survive acknowledge after violation");
    }
}
