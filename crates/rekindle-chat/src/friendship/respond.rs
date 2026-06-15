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
use crate::messaging::VaultSkippedCallback;
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
    peer_profile_key: &str,
    pqxdh_init_message_blob: &[u8],
    our_identity_root: &rekindle_identity::IdentityRoot,
    our_x25519_seed: &[u8; 32],
) -> Result<(), ChatError> {
    // ── Guards ──────────────────────────────────────────────────────

    if pqxdh_init_message_blob.len() > 8192 {
        return Err(ChatError::Deserialization(format!(
            "pqxdh_init_message blob too large ({} bytes, max 8192) — \
             possible malformed entry in DHT inbox",
            pqxdh_init_message_blob.len()
        )));
    }

    // Rate limit: max 1 new session per peer per hour
    if let Ok(Some(last_attempt)) = vault.load_key(
        &format!("pqxdh_last_attempt:{}", &peer_pubkey[..32.min(peer_pubkey.len())]),
    ) {
        if last_attempt.len() == 8 {
            let ts = i64::from_le_bytes(last_attempt[..8].try_into().unwrap_or([0; 8]));
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            if now - ts < 3600 {
                tracing::debug!(
                    peer = &peer_pubkey[..12.min(peer_pubkey.len())],
                    secs_remaining = 3600 - (now - ts),
                    "respond: rate limited — last attempt within 1 hour"
                );
                return Err(ChatError::Internal(format!(
                    "respond: rate limited — {} secs remaining",
                    3600 - (now - ts),
                )));
            }
        }
    }

    if pqxdh_init_message_blob.is_empty() {
        tracing::debug!(
            peer = &peer_pubkey[..12.min(peer_pubkey.len())],
            "empty pqxdh_init_message — mutual accept, session already established"
        );
        return Ok(());
    }

    if session_cache.has_session_for_peer(peer_pubkey)? {
        tracing::debug!(
            peer = &peer_pubkey[..12.min(peer_pubkey.len())],
            "session already exists in cache — skipping respond"
        );
        return Ok(());
    }

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

    let spk_label = rekindle_storage::keys::labels::target_signed_prekey(peer_profile_key);
    let spk_bytes = vault
        .load_key(&spk_label)?
        .ok_or_else(|| ChatError::Internal(format!(
            "signed prekey '{spk_label}' not found in vault — prekey may have been \
             consumed by a previous handshake or the identity was rotated"
        )))?;

    let opk_bytes: Option<Vec<u8>> = None;

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

    // Load PQPK decapsulation key from vault (per-target label, same as request.rs)
    let pqpk_label = rekindle_storage::keys::labels::target_pq_prekey(peer_profile_key);
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
        our_x25519_seed,
        &spk_seed,
        opk_seed.as_ref(),
        &dk,
        &init_msg,
    )?;

    // ── Initialize Double Ratchet as responder ─────────────────────

    // The SPK seed from vault is the private half of the signed prekey
    // that the Initiator used in init_initiator's DH step:
    //   DH(initiator_dr_seed, responder_spk_pub)
    // The Responder computes the commutative counterpart:
    //   DH(responder_spk_seed, initiator_dr_pub)
    let spk_seed_zeroizing = zeroize::Zeroizing::new(spk_seed);
    let spk_pub = rekindle_ratchet::crypto::dh::reusable_from_seed(&spk_seed_zeroizing)
        .map_err(|e| ChatError::Internal(format!("spk pub derive: {e}")))?
        .compute_public_key()
        .map_err(|_| ChatError::Internal("spk pub compute".into()))?;
    let mut spk_pub_bytes = [0u8; 32];
    spk_pub_bytes.copy_from_slice(spk_pub.as_ref());
    let ec_state = DoubleRatchetState::init_responder(
        result.session_key,
        spk_seed_zeroizing,
        spk_pub_bytes,
        init_msg.initiator_ratchet_dh_pub,
    )?;

    let peer_root = rekindle_identity::IdentityRoot::from_hex(peer_pubkey)
        .map_err(|e| ChatError::Internal(format!("peer root from hex: {e}")))?;
    let session_anchor = rekindle_identity::session_anchor(our_identity_root, &peer_root)
        .map_err(|e| ChatError::Internal(format!("session anchor: {e}")))?;
    let session_id_bytes: [u8; 32] = *session_anchor.as_bytes();

    let mut session = TripleRatchetSession::new(
        session_id_bytes,
        Direction::Responder,
        ec_state,
        TrustLevel::TrustOnFirstUse { full_fs: opk_seed.is_some() },
    );

    // ── Verify PQXDH-INIT via EC-level decrypt ──────────────────────
    // The Initiator encrypted b"PQXDH-INIT" via encrypt_he during
    // pqxdh::initiate. decrypt_he is the raw EC counterpart — no
    // hybrid KDF layer. This verifies the Initiator derived the
    // correct session key before we consume one-time prekeys.
    // After decrypt, n_recv advances to 1. cks already initialized (Olm model).
    if !init_msg.initial_encrypted_header.is_empty() && !init_msg.initial_ciphertext.is_empty() {
        let skipped_cb = VaultSkippedCallback {
            vault,
            session_id: &session_id_bytes,
        };
        match rekindle_ratchet::ratchet::ec::decrypt_he(
            &mut session.ec,
            &init_msg.initial_encrypted_header,
            &init_msg.initial_ciphertext,
            &skipped_cb,
        ) {
            Ok(plaintext) => {
                if plaintext.len() != 74 {
                    return Err(ChatError::Internal(format!(
                        "respond: PQXDH-INIT payload wrong length — expected 74, got {}",
                        plaintext.len(),
                    )));
                }
                if &plaintext[..10] != b"PQXDH-INIT" {
                    return Err(ChatError::Internal(
                        "respond: PQXDH-INIT tag mismatch".into(),
                    ));
                }
                if &plaintext[10..42] != &init_msg.ik_a_ed25519[..] {
                    return Err(ChatError::Internal(
                        "respond: PQXDH-INIT sender identity mismatch — possible key-share attack".into(),
                    ));
                }
                let our_signing_seed = vault.require_key(
                    rekindle_storage::keys::labels::SIGNING_KEY,
                ).map_err(|e| ChatError::Internal(format!("signing key: {e}")))?;
                let our_kp = rekindle_identity::SigningKeypair::from_seed(
                    &<[u8; 32]>::try_from(our_signing_seed.as_slice())
                        .map_err(|_| ChatError::Internal("signing seed wrong length".into()))?,
                ).map_err(|e| ChatError::Internal(format!("signing kp: {e}")))?;
                if &plaintext[42..74] != &our_kp.public_key_bytes()[..] {
                    return Err(ChatError::Internal(
                        "respond: PQXDH-INIT recipient identity mismatch — message not intended for us".into(),
                    ));
                }
                tracing::debug!(
                    peer = &peer_pubkey[..12.min(peer_pubkey.len())],
                    n_recv = session.ec.n_recv,
                    "respond: PQXDH-INIT verified — SK possession + identity binding confirmed"
                );
            }
            Err(e) => {
                return Err(ChatError::Internal(format!(
                    "respond: PQXDH-INIT decrypt failed: {e}"
                )));
            }
        }
    }

    // ── Persist session + delete consumed prekeys ──────────────────

    // Persist to vault BEFORE insert — insert takes ownership via Arc<Mutex>.
    // The vault needs peer_key to index the session for ensure_loaded lookups.
    // The CBOR captures post-verify state: n_recv=1 (Olm model — cks initialized at creation).
    let cbor = cbor4ii::serde::to_vec(Vec::new(), &session)
        .map_err(|e| ChatError::Serialization(format!("session CBOR: {e}")))?;
    let direction_byte = match session.direction {
        rekindle_ratchet::Direction::Initiator => 0u8,
        rekindle_ratchet::Direction::Responder => 1u8,
    };

    tracing::debug!(
        peer = &peer_pubkey[..16.min(peer_pubkey.len())],
        session_id = hex::encode(&session_id_bytes[..8]),
        direction = direction_byte,
        cbor_len = cbor.len(),
        "respond: persisting session to vault"
    );

    vault.store_session(
        &session_id_bytes,
        peer_pubkey,
        direction_byte,
        &cbor,
        session.spqr_active,
        0,
    )?;

    tracing::debug!(
        peer = &peer_pubkey[..16.min(peer_pubkey.len())],
        "respond: session persisted to vault — inserting into cache"
    );

    session_cache.insert(session_id_bytes, session).await;

    let _ = vault.delete_key(&spk_label);
    if init_msg.opk_b_id != 0 {
        let opk_label = rekindle_storage::keys::labels::one_time_prekey(init_msg.opk_b_id);
        let _ = vault.delete_key(&opk_label);
    }
    let _ = vault.delete_key(&pqpk_label);

    // Record successful attempt timestamp for rate limiting
    let now_bytes = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64)
        .to_le_bytes();
    let _ = vault.store_key(
        &format!("pqxdh_last_attempt:{}", &peer_pubkey[..32.min(peer_pubkey.len())]),
        &now_bytes,
    );

    // Track last-resort key first use for rotation
    if init_msg.pqpk_is_last_resort {
        let lr_label = format!("pq_lr_first_used:{}", &peer_profile_key[..20.min(peer_profile_key.len())]);
        if vault.load_key(&lr_label).ok().flatten().is_none() {
            let _ = vault.store_key(&lr_label, &now_bytes);
        }
    }

    tracing::info!(
        peer = &peer_pubkey[..16.min(peer_pubkey.len())],
        session_id = hex::encode(&session_id_bytes[..8]),
        spk_id = init_msg.spk_b_id,
        pqpk_id = init_msg.pqpk_b_id,
        "respond: PQXDH responder session established — vault persisted, prekeys deleted"
    );

    Ok(())
}
