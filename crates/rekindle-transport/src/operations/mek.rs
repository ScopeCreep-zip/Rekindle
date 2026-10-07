//! MEK lifecycle operations — request, wrap/unwrap, replenish prekeys.
//!
//! **Rotation is not here.** `rotate_mek` wrapped a fresh key for every
//! member and published the copies into the registry's MEK vault
//! subkey — a write `o_cnt: 0` gives nobody a credential for, of a key
//! `communities-channels.md` says is *"**never** written to DHT"*.
//! Rotation now goes through `rekindle-mek-rotation`, which delivers
//! wrapped keys peer-to-peer by `app_call`: the deterministic rotator on
//! departure, and `daemon::mek_rotation` for an operator request.
//!
//! Typed reads/writes via `dht/profile.rs`. Raw DHT I/O via
//! `broadcast::dht_writes` for profile subkey writes.

use tracing::info;

use crate::broadcast::node::TransportNode;
use crate::crypto::mek::Mek;
use crate::error::{Result, TransportError};

fn parse_pseudonym_pub(hex_str: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| TransportError::Internal(format!("invalid pseudonym hex: {e}")))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| TransportError::Internal("pseudonym key wrong length".into()))?;
    Ok(arr)
}

/// Unwrap a MEK delivered by a rotator.
///
/// Returns the key without caching it: installing a key is protocol
/// policy (refuse a downgrade, keep the lowest election rank at an equal
/// generation — `rekindle_mek_rotation::ChannelMekCache::insert`), which
/// the caller applies.
pub fn unwrap_mek_transfer_payload(
    transfer: &crate::payload::rpc::MekTransferPayload,
    signing_key_bytes: &[u8; 32],
    governance_key: &str,
) -> Result<Mek> {
    let rotator_pub = parse_pseudonym_pub(&transfer.rotator_pseudonym_hex)?;
    let our_pseudonym =
        crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, governance_key);
    let mek_wire =
        crate::crypto::mek::unwrap_mek(&our_pseudonym, &rotator_pub, &transfer.wrapped_mek)?;
    let mek = Mek::from_wire_bytes(&mek_wire).ok_or_else(|| TransportError::MekUnwrapFailed {
        reason: "invalid MEK wire bytes".into(),
    })?;
    if mek.generation() != transfer.generation {
        return Err(TransportError::MekUnwrapFailed {
            reason: format!(
                "wrapped key is generation {}, transfer names {}",
                mek.generation(),
                transfer.generation
            ),
        });
    }
    info!(governance_key, scope = %transfer.scope, generation = mek.generation(), "MEK unwrapped");
    Ok(mek)
}

// `wrap_meks_for_member` lived here: the v1.0 flow where a coordinator
// wrapped every channel MEK for a joining member. It had no callers.
// Under v2.0 the invite carries the MEK in `InviteSecrets` and rotation
// is peer-to-peer (`rekindle-mek-rotation`), so there is no privileged
// peer to do the wrapping — the same reason `node/daemon/mek_wrap.rs`
// went earlier in this migration.

/// Replenish prekeys — generate new bundle, publish to profile DHT via raw primitive.
pub async fn replenish_prekeys(
    node: &TransportNode,
    profile_dht_key: &str,
    signing_key_bytes: &[u8; 32],
) -> Result<u32> {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let verifying_key = signing_key.verifying_key();
    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            signing_key.to_bytes().to_vec(),
            verifying_key.as_bytes().to_vec(),
            1,
        )),
        Box::new(crate::crypto::signal_store::MemoryPreKeyStore::new()),
        Box::new(crate::crypto::signal_store::MemorySessionStore::new()),
    );
    let bundle = signal
        .generate_prekey_bundle(1, Some(1), Some(1))
        .map_err(|e| TransportError::IdentityCreationFailed {
            step: "prekey replenish".into(),
            reason: e.to_string(),
        })?;
    let bundle_bytes = bundle
        .to_bytes()
        .map_err(|e| TransportError::SerializationFailed {
            reason: format!("prekey bundle: {e}"),
        })?;
    let byte_count = bundle_bytes.len();
    crate::broadcast::dht_writes::set_own_profile_subkey(
        node,
        profile_dht_key,
        crate::payload::dht_types::PROFILE_SUBKEY_PREKEY_BUNDLE,
        bundle_bytes,
    )
    .await?;
    let count = u32::try_from(byte_count).unwrap_or(u32::MAX);
    info!(
        bytes = byte_count,
        subkey = crate::payload::dht_types::PROFILE_SUBKEY_PREKEY_BUNDLE,
        "pqxdh_bundle_published kind=LastResort+OneTimeBatch (replenish)",
    );
    Ok(count)
}
