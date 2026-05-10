//! Respond to a Signal session on acceptance discovery (sender side).
//!
//! When the inbox scan discovers an Accepted entry, the sender completes
//! the X3DH handshake by calling `pqxdh::respond` with the handshake
//! fields from the Accepted entry + their prekey private material from
//! the vault.

use zeroize::Zeroizing;

use rekindle_ratchet::crypto::kem;
use rekindle_ratchet::session::{Direction, DoubleRatchetState, TrustLevel, TripleRatchetSession};
use rekindle_storage::VaultStore;

use crate::crypto::sessions::SessionCache;
use crate::ChatError;

/// Complete the PQXDH responder handshake for an accepted friend request.
///
/// Called from inbox scan when an Accepted entry with a non-empty
/// `pqxdh_init_message` blob is discovered. The blob is the JSON-serialized
/// `PqxdhInitMessage` that the acceptor produced via `pqxdh::initiate()`.
///
/// After successful handshake, consumed prekeys are deleted from vault
/// to prevent replay of old Accepted entries.
pub async fn respond_to_acceptance(
    vault: &VaultStore,
    session_cache: &SessionCache,
    peer_pubkey: &str,
    pqxdh_init_message_blob: &[u8],
    our_signing_seed: &[u8; 32],
) -> Result<(), ChatError> {
    // ── Guards ──────────────────────────────────────────────────────

    // Reject oversized blobs before deserialization (DoS protection).
    if pqxdh_init_message_blob.len() > 8192 {
        return Err(ChatError::Deserialization(format!(
            "pqxdh_init_message blob too large ({} bytes, max 8192) — \
             possible malformed entry in DHT inbox",
            pqxdh_init_message_blob.len()
        )));
    }

    if pqxdh_init_message_blob.is_empty() {
        tracing::debug!(
            peer = &peer_pubkey[..12.min(peer_pubkey.len())],
            "empty pqxdh_init_message — mutual accept, session already established"
        );
        return Ok(());
    }

    // Check in-memory session cache (hot path).
    if session_cache.has_session_for_peer(peer_pubkey)? {
        tracing::debug!(
            peer = &peer_pubkey[..12.min(peer_pubkey.len())],
            "session already exists in cache — skipping respond"
        );
        return Ok(());
    }

    // Check vault for existing session (cold path — daemon restart case).
    // Without this, replaying an old Accepted entry after restart would
    // re-derive the session and overwrite the active one.
    if vault.load_session_by_peer(peer_pubkey)?.is_some() {
        tracing::debug!(
            peer = &peer_pubkey[..12.min(peer_pubkey.len())],
            "session already exists in vault — skipping respond"
        );
        return Ok(());
    }

    // ── Deserialize the PQXDH init message from the blob ───────────

    let init_msg: rekindle_ratchet::pqxdh::bundle::PqxdhInitMessage =
        serde_json::from_slice(pqxdh_init_message_blob)
            .map_err(|e| ChatError::Deserialization(format!(
                "pqxdh_init_message deserialize failed: {e}"
            )))?;

    // ── Load our prekey private material from vault ─────────────────

    let spk_label = rekindle_storage::keys::labels::signed_prekey(init_msg.spk_b_id);
    let spk_bytes = vault
        .load_key(&spk_label)?
        .ok_or_else(|| ChatError::Internal(format!(
            "signed prekey '{spk_label}' not found in vault — prekey may have been \
             consumed by a previous handshake or the identity was rotated"
        )))?;

    let opk_bytes = if init_msg.opk_b_id != 0 {
        let label = rekindle_storage::keys::labels::one_time_prekey(init_msg.opk_b_id);
        vault.load_key(&label)?
    } else {
        None
    };

    // Load our identity X25519 DH seed
    let ik_dh_seed_bytes = vault
        .load_key(rekindle_storage::keys::labels::IDENTITY_X25519_SEED)?
        .ok_or_else(|| ChatError::Internal("X25519 identity seed not in vault".into()))?;

    if ik_dh_seed_bytes.len() != 32 {
        return Err(ChatError::Internal("X25519 seed wrong length".into()));
    }
    let mut ik_dh_seed = [0u8; 32];
    ik_dh_seed.copy_from_slice(&ik_dh_seed_bytes);

    if spk_bytes.len() != 32 {
        return Err(ChatError::Internal("SPK seed wrong length".into()));
    }
    let mut spk_seed = [0u8; 32];
    spk_seed.copy_from_slice(&spk_bytes);

    let opk_seed = opk_bytes.and_then(|b| {
        if b.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            Some(arr)
        } else {
            None
        }
    });

    // Load PQPK decapsulation key from vault
    let pqpk_label = rekindle_storage::keys::labels::pq_prekey(init_msg.pqpk_b_id);
    let dk_bytes_vec = vault
        .load_key(&pqpk_label)?
        .ok_or_else(|| ChatError::Internal(format!(
            "PQ prekey '{pqpk_label}' not in vault — may have been consumed or rotated"
        )))?;

    if dk_bytes_vec.len() != kem::DK_LEN {
        return Err(ChatError::Internal(format!(
            "PQ dk wrong length: {} (expected {})", dk_bytes_vec.len(), kem::DK_LEN
        )));
    }
    let mut dk = Zeroizing::new([0u8; kem::DK_LEN]);
    dk.copy_from_slice(&dk_bytes_vec);

    // ── Run PQXDH responder ────────────────────────────────────────

    let result = rekindle_ratchet::pqxdh::respond(
        &ik_dh_seed,
        &spk_seed,
        opk_seed.as_ref(),
        &dk,
        &init_msg,
    )?;

    // ── Initialize Double Ratchet as responder ─────────────────────

    let (our_dh_seed, our_dh_pub) = rekindle_ratchet::crypto::dh::generate_ratchet_keypair()?;
    let ec_state = DoubleRatchetState::init_responder(
        result.session_key,
        our_dh_seed,
        our_dh_pub,
    )?;

    // Session ID = BLAKE3(our_signing_seed || peer_pubkey_bytes)
    let session_id = blake3::hash(
        &[our_signing_seed.as_slice(), peer_pubkey.as_bytes()].concat(),
    );
    let session_id_bytes: [u8; 32] = *session_id.as_bytes();

    let session = TripleRatchetSession::new(
        session_id_bytes,
        Direction::Responder,
        ec_state,
        TrustLevel::TrustOnFirstUse { full_fs: opk_seed.is_some() },
    );

    // ── Persist session + delete consumed prekeys ──────────────────

    session_cache.insert(session_id_bytes, session).await;

    // Delete consumed prekeys from vault. These are single-use —
    // replaying an old Accepted entry after this point will fail at
    // the vault.load_key step above ("not found in vault").
    let _ = vault.delete_key(&spk_label);
    if init_msg.opk_b_id != 0 {
        let opk_label = rekindle_storage::keys::labels::one_time_prekey(init_msg.opk_b_id);
        let _ = vault.delete_key(&opk_label);
    }
    let _ = vault.delete_key(&pqpk_label);

    tracing::info!(
        peer = &peer_pubkey[..12.min(peer_pubkey.len())],
        spk_id = init_msg.spk_b_id,
        pqpk_id = init_msg.pqpk_b_id,
        "PQXDH responder session established — consumed prekeys deleted"
    );

    Ok(())
}
