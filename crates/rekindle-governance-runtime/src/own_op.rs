//! Our op, signed (plan E3.2, ADR 0022 rules 1 and 3): the apply
//! pipeline's signing seam (`crate::apply`), kept off the async path so the
//! device key and the pseudonym's secret never live across an await.

use rekindle_governance::op::VerifiedOp;
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::derive;
use rekindle_types::governance::{GenesisCert, GovernanceEntry};
use rekindle_types::id::PseudonymKey;
use zeroize::Zeroizing;

use crate::deps::GovernanceRuntimeDeps;
use crate::device_signer;
use crate::error::GovernanceRuntimeError;
use crate::op_log;

/// What our op is built on: the community, its pinned cert, the locked
/// state and the verified op set, and our pseudonym `p` (its author).
pub(crate) struct Ours<'a> {
    pub(crate) community_id: &'a str,
    pub(crate) cert: &'a GenesisCert,
    pub(crate) state: &'a GovernanceState,
    pub(crate) set: &'a op_log::VerifiedSet,
    pub(crate) author: PseudonymKey,
}

/// Our op for `entry`, signed by this device's op key under `p` (ADR 0022
/// rule 1); a `MemberJoined`, this device's first op here, gets `p`'s
/// endorsement of the key written into `entry` (rule 3). Kept off
/// the async path: the device key and `p`'s secret never live across an
/// await, so `write_with`'s future holds neither.
///
/// # Errors
/// No identity secret, no stored device key for a later op, or
/// [`op_log::build_op`]'s.
pub(crate) fn sign_ours<D: GovernanceRuntimeDeps>(
    deps: &D,
    ours: Ours<'_>,
    entry: &mut GovernanceEntry,
    op_deps: op_log::Deps,
) -> Result<(VerifiedOp, Vec<u8>), GovernanceRuntimeError> {
    let identity_secret = Zeroizing::new(
        deps.identity_secret()
            .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?,
    );
    let key = derive::derive_community_pseudonym(&identity_secret, ours.community_id);
    let first_op = matches!(entry, GovernanceEntry::MemberJoined { .. });
    let device = device_signer::device_op_key(deps, ours.community_id, first_op)?;
    device_signer::carry_first_device(entry, &key, ours.community_id, &device, ours.state);
    let signer =
        device_signer::OpSigner::checked(ours.community_id, ours.state, ours.author, &device)?;
    op_log::build_op(
        ours.community_id,
        ours.cert,
        ours.state,
        ours.set,
        signer,
        entry.clone(),
        op_deps,
        deps.clock().now_secs(),
    )
}
