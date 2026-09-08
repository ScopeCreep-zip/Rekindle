//! Who is a member, derived the way the architecture defines it.
//!
//! There is no membership ledger to read. `communities-overview.md`
//! describes the registry as *"Single struct overwritten per heartbeat.
//! Never grows"* — a presence directory — and under
//! [`AdmissionMode::Open`](rekindle_types::governance::AdmissionMode)
//! governance carries no entry for a joiner either, because they claim
//! a slot without asking. So membership is not stored anywhere as a
//! list; it is a **predicate over registry rows**:
//!
//! > a member is someone holding a slot with a validly-signed
//! > `MemberPresence` who is neither banned nor departed.
//!
//! That is the same predicate [`crate::join_stages::reclaim`] applies
//! from the other side — it asks which slots *fail* it — and both run
//! through `rekindle_presence::parse_and_classify_row` so the W26
//! signature check and ban filter cannot drift between "who counts as a
//! member" and "whose slot can be taken".
//!
//! ## Why this is not free
//!
//! It costs one fetch per occupied slot. The v1.0 member index answered
//! the same question in a single read *because it was an aggregate* —
//! one member's row asserting facts about everyone. `o_cnt: 0` gives
//! nobody the authority to write that, which is precisely why it went.
//! The exact count is per-slot or it is not trustworthy.
//!
//! So this is for **one-shot** questions — login hydration recovering
//! our own slot, a join deciding which slots are reusable. Anything
//! asked per request reads the roster the presence poll already
//! materialised; re-deriving it per query does the validation twice and
//! is the reason `CommunityPresenceDeps` exists.

use std::collections::HashSet;

use rekindle_governance::state::GovernanceState;
use rekindle_presence::{parse_and_classify_row, ClassifiedRow};

use crate::deps::GovernanceRuntimeDeps;
use crate::join_stages::registry_scan;

/// One validated member found in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterMember {
    /// The slot they hold.
    pub subkey: u32,
    /// Their community pseudonym, hex-encoded.
    pub pseudonym_hex: String,
    /// Self-declared display name, if they set one. Peer-supplied and
    /// unvalidated beyond the signature — render escaped.
    pub display_name: Option<String>,
}

/// Every member holding a slot in one registry segment.
///
/// Offline members are included: absence is not departure, and a member
/// away for a month still holds their slot. Only a signed departure, a
/// ban, or an unverifiable row removes someone.
pub async fn segment_roster<D: GovernanceRuntimeDeps>(
    deps: &D,
    registry_key: &str,
    gov_state: &GovernanceState,
) -> Vec<RosterMember> {
    let occupied = match deps.inspect_dht_record_present_subkeys(registry_key).await {
        Ok(subkeys) => subkeys,
        Err(error) => {
            tracing::debug!(
                registry = %registry_key,
                %error,
                "roster: inspect failed; treating segment as unreadable rather than empty"
            );
            return Vec::new();
        }
    };

    let banned: HashSet<String> = gov_state.bans.iter().map(|p| hex::encode(p.0)).collect();

    // Under `ApprovalRequired`, holding a slot is not membership.
    //
    // This is where admission is actually enforced. `o_cnt: 0` means the
    // SMPL schema cannot refuse a write, so anyone with the slot seed
    // can occupy a subkey whatever the mode says — the architecture is
    // explicit that "an unapproved member still occupies an SMPL slot;
    // the schema cannot prevent that, but honest peers do not count them
    // as a member". Every reader applies this independently, exactly as
    // it applies the ban list.
    //
    // Without it `ApprovalRequired` was inert: the mode was recorded at
    // genesis, `approve_member` wrote decisions, and nothing ever read
    // them.
    let approval_required = gov_state.effective_admission_mode()
        == rekindle_types::governance::AdmissionMode::ApprovalRequired;

    registry_scan::fetch_occupied(deps, registry_key, &occupied)
        .await
        .into_iter()
        .filter_map(|(subkey, raw)| {
            // Staleness arguments are inert: `parse_and_classify_row`
            // uses them only to decide whether an accepted row is
            // *online*, never whether it is accepted.
            match parse_and_classify_row(&raw, &banned, 0, 0) {
                ClassifiedRow::Accepted(row) => {
                    if approval_required && !is_admitted(gov_state, &row.pseudonym_hex) {
                        return None;
                    }
                    Some(RosterMember {
                        subkey,
                        pseudonym_hex: row.pseudonym_hex,
                        display_name: row.presence.display_name,
                    })
                }
                // Empty, malformed, forged, banned, departed — none of
                // these is a member. `reclaim` treats the same set as
                // reusable slots.
                _ => None,
            }
        })
        .collect()
}

/// Has this pseudonym been admitted?
///
/// The creator is admitted by construction — they wrote the genesis
/// entries, so requiring an approval they could only give themselves
/// would lock them out of their own community on the first merge.
fn is_admitted(gov_state: &GovernanceState, pseudonym_hex: &str) -> bool {
    let key = rekindle_types::id::PseudonymKey::from_hex_lossy(pseudonym_hex);
    if gov_state.creator.as_ref() == Some(&key) {
        return true;
    }
    gov_state
        .admitted
        .get(&key)
        .is_some_and(|decision| decision.approved)
}

/// The slot we occupy in a registry, found by our own pseudonym.
///
/// Replaces reading a member index row to recover `my_subkey_index`.
/// Stronger than what it replaces: the index was a shared structure any
/// member could write, so a row claiming our slot index proved nothing.
/// Here the row must carry a signature over our own pseudonym, which
/// only we can produce.
pub async fn find_my_slot<D: GovernanceRuntimeDeps>(
    deps: &D,
    registry_key: &str,
    my_pseudonym_hex: &str,
    gov_state: &GovernanceState,
) -> Option<u32> {
    segment_roster(deps, registry_key, gov_state)
        .await
        .into_iter()
        .find(|m| m.pseudonym_hex == my_pseudonym_hex)
        .map(|m| m.subkey)
}

#[cfg(test)]
mod tests {
    use super::is_admitted;
    use rekindle_governance::state::{AdmissionDecision, GovernanceState};
    use rekindle_types::governance::AdmissionMode;
    use rekindle_types::id::PseudonymKey;

    fn key(byte: u8) -> PseudonymKey {
        PseudonymKey([byte; 32])
    }
    fn hex_of(byte: u8) -> String {
        hex::encode([byte; 32])
    }

    #[test]
    fn an_approved_member_is_admitted() {
        let mut state = GovernanceState::default();
        state.admitted.insert(
            key(1),
            AdmissionDecision {
                approved: true,
                lamport: 4,
            },
        );
        assert!(is_admitted(&state, &hex_of(1)));
    }

    /// A rejected member holds a slot — `o_cnt: 0` cannot stop them —
    /// but is not counted. This is the whole enforcement point.
    #[test]
    fn a_rejected_member_is_not_admitted() {
        let mut state = GovernanceState::default();
        state.admitted.insert(
            key(2),
            AdmissionDecision {
                approved: false,
                lamport: 4,
            },
        );
        assert!(!is_admitted(&state, &hex_of(2)));
    }

    #[test]
    fn an_undecided_member_is_not_admitted() {
        assert!(!is_admitted(&GovernanceState::default(), &hex_of(3)));
    }

    /// The creator wrote the genesis entries, so requiring an approval
    /// they could only give themselves would lock them out of their own
    /// community on the first merge.
    #[test]
    fn the_creator_is_admitted_without_a_decision() {
        let state = GovernanceState {
            creator: Some(key(9)),
            ..Default::default()
        };
        assert!(state.admitted.is_empty());
        assert!(is_admitted(&state, &hex_of(9)));
    }

    /// Default is `Open` — every community that predates the admission
    /// entries must keep admitting everyone, or the roster would empty
    /// itself on upgrade.
    #[test]
    fn absent_admission_policy_means_open() {
        let state = GovernanceState::default();
        assert_eq!(state.effective_admission_mode(), AdmissionMode::Open);
    }
}
