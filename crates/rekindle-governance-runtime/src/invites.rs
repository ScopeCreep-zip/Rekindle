//! Minting an invite, once, for both tracks.
//!
//! An invite is not a code — it is a code plus an encrypted blob
//! carrying everything a joiner cannot derive: the registry key, the
//! **shared slot seed**, the current MEK, and the channel record keys.
//! The blob lives in its own DFLT record and governance carries only a
//! pointer, because it is multi-KB and would overflow the per-subkey
//! SMPL cap if inlined.
//!
//! ## What the daemon was doing instead
//!
//! `transport::operations::invites::create_invite` appended an
//! `InviteEntry` to the **v1.0 governance-manifest invites subkey** with
//! `encrypted_secrets: None`, and wrote no `InviteCreated` entry. Two
//! consequences, both silent:
//!
//! * The invite could not be used at all. `join_flow::decode_invite`
//!   needs a secrets pointer to reach the slot seed — the registry's 255
//!   member keys come from the creator's seed, so a joiner that cannot
//!   read it can never produce a matching writer. It failed with
//!   "invite has no secrets pointer".
//! * With no `InviteCreated` in the CRDT there was nothing for
//!   `inspect_invite_in_entries` to find, so revocation, expiry and the
//!   per-inviter `invite_quota` cap never applied to it.
//!
//! The quota is the slot-exhaustion control
//! (`rekindle_governance::invite_quota`), which is why the second point
//! matters as much as the first.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::invite::{ChannelKeyInfo, InviteSecrets};

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::invite_secrets::publish_invite_secrets;

/// A freshly minted invite.
#[derive(Debug, Clone)]
pub struct MintedInvite {
    /// The raw code. Shown to the user once; only its hash is published.
    pub code: String,
    pub code_hash: String,
    /// Where the encrypted secrets landed — the joiner needs this and
    /// the code together.
    pub secrets_record_key: String,
    pub invite_id: [u8; 16],
    pub expires_at: Option<u64>,
}

/// Mint an invite and announce it in governance.
pub async fn create_invite<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    max_uses: u32,
    expires_in_seconds: Option<u64>,
) -> Result<MintedInvite, GovernanceRuntimeError> {
    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;
    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;

    let registry_key = membership.member_registry_key.clone().ok_or_else(|| {
        GovernanceRuntimeError::Adapter("no registry key for community".to_string())
    })?;
    let slot_seed = membership
        .slot_seed_hex
        .clone()
        .ok_or_else(|| GovernanceRuntimeError::SlotSeedMissing(community_id.to_string()))?;

    // Architecture §6.2 — an invite must bootstrap the joiner into the
    // PRIMARY (segment-0) registry: the joiner treats the invite's
    // registry as `slot_range_start 0` and derives its slot keypair from
    // that. Segment 0 is implicit and never listed in
    // `gov_state.segments`, so a match against a segment-≥1 registry
    // means the value is corrupt and the invite would mis-route every
    // joiner who used it. Refuse rather than ship the break.
    if let Some(segment) = gov_state
        .segments
        .iter()
        .find(|s| s.segment_index != 0 && s.registry_key == registry_key)
    {
        return Err(GovernanceRuntimeError::Adapter(format!(
            "invite registry key is the segment-{} registry, not the primary",
            segment.segment_index
        )));
    }

    let mek = deps.community_mek(community_id).ok_or_else(|| {
        GovernanceRuntimeError::Adapter("no MEK available to seed the invite".to_string())
    })?;
    let mek_wire_bytes = {
        use base64::Engine as _;
        let mut wire = Vec::with_capacity(40);
        wire.extend_from_slice(&mek.generation.to_le_bytes());
        wire.extend_from_slice(&mek.key_bytes);
        base64::engine::general_purpose::STANDARD.encode(wire)
    };

    // Channel record keys from merged governance — the joiner reads
    // history from these directly rather than waiting for its own merge.
    let channel_keys: Vec<ChannelKeyInfo> = gov_state
        .channels
        .iter()
        .map(|(channel_id, channel)| ChannelKeyInfo {
            channel_id: hex::encode(channel_id.0),
            record_key: channel.record_key.clone(),
            name: channel.name.clone(),
        })
        .collect();

    let code = hex::encode(rekindle_utils::random::id_bytes_16());
    let code_hash = rekindle_secrets::invite::hash_invite_code(&code);

    let secrets = InviteSecrets {
        governance_key: membership
            .governance_key
            .clone()
            .unwrap_or_else(|| community_id.to_string()),
        registry_key,
        inviter_route_blob: deps.our_route_blob(),
        slot_seed,
        mek_wire_bytes,
        channel_keys,
        // From merged governance, not the local membership record: the
        // name a joiner is shown before joining should be the one the
        // community agrees on, not whatever this peer cached.
        community_name: gov_state
            .metadata
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_default(),
    };
    let secrets_json = serde_json::to_vec(&secrets)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("serialize invite secrets: {e}")))?;
    let encrypted = rekindle_secrets::invite::encrypt_invite_secrets(&code, &secrets_json)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("encrypt invite secrets: {e}")))?;
    let encrypted_b64 = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&encrypted)
    };

    let secrets_record_key = publish_invite_secrets(deps, &encrypted_b64).await?;

    let invite_id = rekindle_utils::random::id_bytes_16();
    let expires_at = expires_in_seconds.map(|s| rekindle_utils::timestamp_secs() + s);
    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::InviteCreated {
            invite_id,
            code_hash: code_hash.clone(),
            max_uses,
            expires_at,
            secrets_record_key: secrets_record_key.clone(),
            lamport,
        },
    )
    .await?;

    Ok(MintedInvite {
        code,
        code_hash,
        secrets_record_key,
        invite_id,
        expires_at,
    })
}

/// Revoke an invite by writing the tombstone every peer merges.
///
/// The v1.0 path rewrote the manifest invites subkey — a community-wide
/// entry `o_cnt: 0` credentials nobody for, and one no reader consults.
pub async fn revoke_invite<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    invite_id: [u8; 16],
) -> Result<(), GovernanceRuntimeError> {
    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::InviteRevoked { invite_id, lamport },
    )
    .await
}
