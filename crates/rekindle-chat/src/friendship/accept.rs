//! Accept a pending friend request — PQXDH handshake, DhtLog setup, inbox write.

use rekindle_ratchet::crypto::sign;
use rekindle_ratchet::session::{Direction, TrustLevel, TripleRatchetSession};
use rekindle_storage::keys::labels;
use rekindle_types::transport::RecordSchema;
use rekindle_types::dht_types::{
    FriendEntry, FriendRequestEntry, FriendRequestStatus,
    PROFILE_SUBKEY_FRIEND_INBOX_KEY, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
};
use rekindle_types::session_types::DmPeerLog;

use crate::ChatError;
use crate::time::{timestamp_ms, timestamp_secs};
use super::FriendshipService;
use super::request::blake3_hash_mod;

/// Result of accepting a friend request.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FriendAccepted {
    pub peer_key: String,
    pub outbound_log: String,
    pub inbound_log: String,
    pub mutual: bool,
}

impl FriendshipService {
    /// Accept a pending friend request.
    ///
    /// 1. Load pending request from session meta
    /// 2. Check for mutual acceptance (they already accepted our request)
    /// 3. If not mutual: PQXDH initiate → establish Triple Ratchet session
    /// 4. Create our outbound DhtLog
    /// 5. Adopt sender's DhtLog as our inbound
    /// 6. Write Accepted entry to requester's inbox
    /// 7. Persist session + keypairs + dm_peers to vault
    /// 8. Establish DM watch on inbound log
    pub async fn accept_friend_request(
        &self,
        peer_pubkey: &str,
    ) -> Result<FriendAccepted, ChatError> {
        let signing_key_bytes = self.io.require_signing_key()?;
        let identity = self.require_identity()?;

        // Already friends check
        {
            let meta = self.session_meta.read();
            if let Some(peer_log) = meta.dm_peers.get(peer_pubkey) {
                if !peer_log.outbound_log_key.is_empty() && !peer_log.inbound_log_key.is_empty() {
                    return Ok(FriendAccepted {
                        peer_key: peer_pubkey.to_string(),
                        outbound_log: peer_log.outbound_log_key.clone(),
                        inbound_log: peer_log.inbound_log_key.clone(),
                        mutual: true,
                    });
                }
            }
        }

        // Load the pending request
        let request = {
            let meta = self.session_meta.read();
            meta.pending_request_by_key(peer_pubkey)
                .cloned()
                .ok_or_else(|| ChatError::RequestNotFound {
                    peer_key: peer_pubkey.to_string(),
                })?
        };

        // Check for mutual case: they already accepted OUR request.
        // Read our inbox subkey where they would have written an Accepted entry.
        let mutual_result = self
            .check_mutual_acceptance(peer_pubkey, &identity)
            .await?;
        if let Some(accepted) = mutual_result {
            return Ok(accepted);
        }

        // PQXDH initiate using their prekey bundle
        let kp = sign::keypair_from_seed(&signing_key_bytes)?;
        let our_ed_pub = sign::public_key_bytes(&kp);

        // Deserialize the peer's PreKeyBundle from the pending request
        let peer_bundle: rekindle_ratchet::pqxdh::bundle::PreKeyBundle =
            serde_json::from_slice(&request.prekey_bundle)
                .map_err(|e| ChatError::Deserialization(format!("peer prekey bundle: {e}")))?;

        // Validate the bundle (signature verification, length checks, freshness)
        let now_secs = timestamp_secs();
        rekindle_ratchet::pqxdh::verify::validate_bundle(&peer_bundle, now_secs)?;

        // Derive our X25519 DH identity seed (separate from Ed25519)
        let ik_dh_seed = blake3::derive_key("rekindle identity x25519 v1", &signing_key_bytes);

        // Run PQXDH initiator handshake — generates EK_A internally,
        // performs DH1-DH4 + ML-KEM encaps, derives session key,
        // initializes DR state, produces first encrypted message
        let pqxdh_result = rekindle_ratchet::pqxdh::initiate(
            &ik_dh_seed,
            &our_ed_pub,
            &peer_bundle,
        )?;

        let ec_state = pqxdh_result.ec_state;

        // Serialize the PQXDH init message for the Accepted entry.
        // The sender (requester) will deserialize this blob and pass it to
        // pqxdh::respond() to derive the same session key.
        let pqxdh_init_message_bytes = serde_json::to_vec(&pqxdh_result.init_message)
            .map_err(|e| ChatError::Serialization(format!("pqxdh init message: {e}")))?;

        // Session ID = BLAKE3(IK_A || IK_B)
        let session_id = blake3::hash(
            &[our_ed_pub.as_slice(), peer_pubkey.as_bytes()].concat(),
        );
        let session_id_bytes: [u8; 32] = *session_id.as_bytes();

        let trust_level = TrustLevel::TrustOnFirstUse { full_fs: false };
        let session = TripleRatchetSession::new(
            session_id_bytes,
            Direction::Initiator,
            ec_state,
            trust_level,
        );

        // Persist session to vault
        self.session_cache.insert(session_id_bytes, session).await;

        // Create our outbound DhtLog
        let (outbound_key, outbound_keypair) = self
            .io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let outbound_keypair_hex = hex::encode(&outbound_keypair);

        // Adopt sender's DhtLog as our inbound
        let inbound_key = request.dm_log_key.clone();
        let inbound_keypair_bytes = hex::decode(&request.dm_log_keypair_hex)
            .map_err(|e| ChatError::Internal(format!("inbound keypair hex: {e}")))?;

        // Store keypairs in vault
        let out_short = &outbound_key[..12.min(outbound_key.len())];
        self.vault.store_key(&labels::dm_log_keypair(out_short), &outbound_keypair)?;
        let in_short = &inbound_key[..12.min(inbound_key.len())];
        self.vault.store_key(&labels::dm_log_keypair(in_short), &inbound_keypair_bytes)?;

        // Write Accepted entry to requester's inbox
        self.write_acceptance_to_inbox(
            &identity,
            &signing_key_bytes,
            &request,
            &outbound_key,
            &outbound_keypair_hex,
            &pqxdh_init_message_bytes,
        )
        .await?;

        // Update session meta
        {
            let mut meta = self.session_meta.write();
            meta.friend_display_names
                .insert(peer_pubkey.to_string(), request.display_name.clone());
            meta.remove_pending_friend_request(peer_pubkey);
            meta.pending_outbound_logs.remove(&request.profile_dht_key);
            meta.dm_peers.insert(
                peer_pubkey.to_string(),
                DmPeerLog {
                    outbound_log_key: outbound_key.clone(),
                    inbound_log_key: inbound_key.clone(),
                },
            );
        }

        // Persist friend display name and pending outbound cleanup to vault
        self.vault.store_friend_name(peer_pubkey, &request.display_name)?;

        // Persist FriendEntry to the friend list DHT record for durability.
        // If the vault is lost, the friend list can be recovered from this DHT record.
        let friend_entry = FriendEntry {
            public_key: peer_pubkey.to_string(),
            nickname: None,
            group: None,
            added_at: timestamp_ms(),
            profile_dht_key: Some(request.profile_dht_key.clone()),
            dm_log_key: Some(outbound_key.clone()),
        };
        let fl_keypair = self.vault.load_key(
            rekindle_storage::keys::labels::FRIEND_LIST_KEYPAIR,
        )?;
        if let Some(ref kp) = fl_keypair {
            let fl_key = {
                let meta = self.session_meta.read();
                meta.identity.as_ref().map(|i| i.friend_list_dht_key.clone()).unwrap_or_default()
            };
            if !fl_key.is_empty() {
                let existing = self.io.read_record(&fl_key, 0, false).await?.unwrap_or_default();
                let mut friend_list: rekindle_types::dht_types::FriendList =
                    if existing.is_empty() || existing == b"[]" {
                        rekindle_types::dht_types::FriendList::default()
                    } else {
                        serde_json::from_slice(&existing).unwrap_or_default()
                    };
                friend_list.friends.retain(|f| f.public_key != peer_pubkey);
                friend_list.friends.push(friend_entry);
                let bytes = serde_json::to_vec(&friend_list)
                    .map_err(|e| ChatError::Serialization(format!("friend list: {e}")))?;
                if let Err(e) = self.io.write_record(&fl_key, 0, &bytes, Some(kp), crate::io::Confirm::Accepted).await {
                    tracing::warn!(error = %e, "friend list DHT write failed — local vault copy is authoritative");
                }
            }
        }

        // Watch inbound log for DM receipt
        if let Err(e) = self.io.watch_and_register(
            &inbound_key, &[0],
            crate::events::registry::WatchKind::DmLog { peer_key: peer_pubkey.to_string() },
            &self.watches,
        ).await {
            tracing::warn!(
                peer = &peer_pubkey[..12.min(peer_pubkey.len())],
                error = %e,
                "DM watch failed after accept — messages will arrive via poll"
            );
        }

        tracing::info!(
            peer = &peer_pubkey[..16.min(peer_pubkey.len())],
            outbound = %outbound_key,
            inbound = %inbound_key,
            "friend request accepted"
        );

        Ok(FriendAccepted {
            peer_key: peer_pubkey.to_string(),
            outbound_log: outbound_key,
            inbound_log: inbound_key,
            mutual: false,
        })
    }

    /// Check if the peer already accepted OUR request (mutual case).
    async fn check_mutual_acceptance(
        &self,
        peer_pubkey: &str,
        identity: &rekindle_types::session_types::SessionIdentity,
    ) -> Result<Option<FriendAccepted>, ChatError> {
        let target_subkey = blake3_hash_mod(
            peer_pubkey,
            &identity.profile_dht_key,
            32,
        );

        self.io
            .open_record(&identity.friend_inbox_key, None)
            .await?;

        let data = self
            .io
            .read_record(&identity.friend_inbox_key, target_subkey, true)
            .await?;

        let Some(data) = data else { return Ok(None) };
        if data.is_empty() || data == b"[]" {
            return Ok(None);
        }

        let entries = FriendRequestEntry::parse_inbox_data(&data).unwrap_or_default();

        let accepted_entry = entries.iter().find(|e| {
            e.sender_public_key == peer_pubkey
                && matches!(e.status, FriendRequestStatus::Accepted { .. })
        });

        let Some(entry) = accepted_entry else {
            return Ok(None);
        };

        let FriendRequestStatus::Accepted {
            ref responder_outbound_log_key,
            ref pqxdh_init_message,
            ..
        } = entry.status
        else {
            return Ok(None);
        };

        tracing::info!(
            peer = &peer_pubkey[..16.min(peer_pubkey.len())],
            "mutual acceptance detected — they already accepted our request"
        );

        // Complete PQXDH handshake — the peer ran initiate(), we run respond().
        // This establishes the Triple Ratchet session for DM encryption.
        if !pqxdh_init_message.is_empty() {
            let signing_seed = self.io.require_signing_key()?;
            if let Err(e) = crate::friendship::respond::respond_to_acceptance(
                &self.vault,
                &self.session_cache,
                peer_pubkey,
                pqxdh_init_message,
                &signing_seed,
            ).await {
                tracing::warn!(
                    peer = &peer_pubkey[..12.min(peer_pubkey.len())],
                    error = %e,
                    "mutual PQXDH respond failed — session not established. \
                     DMs will fail until next inbox scan retry."
                );
            }
        }

        // Recover our outbound from pending_outbound_logs
        let our_outbound = self
            .vault
            .take_pending_outbound(&entry.profile_dht_key)?
            .unwrap_or_default();

        // Update session meta
        {
            let mut meta = self.session_meta.write();
            let display_name = meta
                .pending_request_by_key(peer_pubkey)
                .map(|r| r.display_name.clone())
                .unwrap_or_default();
            meta.remove_pending_friend_request(peer_pubkey);
            if !display_name.is_empty() {
                meta.friend_display_names
                    .insert(peer_pubkey.to_string(), display_name.clone());
                let _ = self.vault.store_friend_name(peer_pubkey, &display_name);
            }
            let peer_log = meta
                .dm_peers
                .entry(peer_pubkey.to_string())
                .or_insert_with(|| DmPeerLog {
                    outbound_log_key: String::new(),
                    inbound_log_key: String::new(),
                });
            if !our_outbound.is_empty() {
                peer_log.outbound_log_key.clone_from(&our_outbound);
            }
            peer_log
                .inbound_log_key
                .clone_from(responder_outbound_log_key);
        }

        // Watch inbound log
        if let Err(e) = self.io.watch_and_register(
            responder_outbound_log_key, &[0],
            crate::events::registry::WatchKind::DmLog { peer_key: peer_pubkey.to_string() },
            &self.watches,
        ).await {
            tracing::warn!(
                peer = &peer_pubkey[..12.min(peer_pubkey.len())],
                error = %e,
                "DM watch failed in mutual accept — messages will arrive via poll"
            );
        }

        Ok(Some(FriendAccepted {
            peer_key: peer_pubkey.to_string(),
            outbound_log: our_outbound,
            inbound_log: responder_outbound_log_key.clone(),
            mutual: true,
        }))
    }

    /// Write an Accepted entry to the requester's friend inbox.
    async fn write_acceptance_to_inbox(
        &self,
        identity: &rekindle_types::session_types::SessionIdentity,
        signing_key: &[u8; 32],
        request: &rekindle_types::session_types::PendingFriendRequest,
        outbound_key: &str,
        outbound_keypair_hex: &str,
        pqxdh_init_message_bytes: &[u8],
    ) -> Result<(), ChatError> {
        // Read requester's inbox key from their profile
        self.io
            .open_record(&request.profile_dht_key, None)
            .await?;

        let req_inbox_key = self
            .io
            .read_record(&request.profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY, true)
            .await?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        let req_inbox_kp = self
            .io
            .read_record(&request.profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR, true)
            .await?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();

        if req_inbox_key.is_empty() || req_inbox_kp.is_empty() {
            return Err(ChatError::InboxNotAvailable);
        }

        let kp_bytes = hex::decode(&req_inbox_kp)
            .map_err(|e| ChatError::Internal(format!("requester inbox kp: {e}")))?;

        self.io
            .open_record(&req_inbox_key, Some(&kp_bytes))
            .await?;

        let kp = sign::keypair_from_seed(signing_key)?;

        let mut response = FriendRequestEntry {
            sender_public_key: identity.public_key_hex.clone(),
            display_name: identity.display_name.clone(),
            message: String::new(),
            profile_dht_key: identity.profile_dht_key.clone(),
            mailbox_dht_key: identity.mailbox_dht_key.clone(),
            sender_friend_inbox_key: identity.friend_inbox_key.clone(),
            sender_friend_inbox_keypair_hex: identity.friend_inbox_keypair_hex.clone(),
            prekey_bundle: Vec::new(),
            sent_at: timestamp_ms(),
            dm_log_key: outbound_key.to_string(),
            dm_log_keypair_hex: outbound_keypair_hex.to_string(),
            x25519_pub_hex: String::new(),
            signature_hex: String::new(),
            status: FriendRequestStatus::Accepted {
                responder_profile_dht_key: identity.profile_dht_key.clone(),
                responder_mailbox_dht_key: identity.mailbox_dht_key.clone(),
                responder_outbound_log_key: outbound_key.to_string(),
                responder_outbound_log_keypair_hex: outbound_keypair_hex.to_string(),
                pqxdh_init_message: pqxdh_init_message_bytes.to_vec(),
                accepted_at: timestamp_ms(),
            },
        };

        let content = response.signature_content();
        let sig = sign::sign_ec_prekey(&kp, &content);
        response.signature_hex = hex::encode(sig);

        let subkey = blake3_hash_mod(
            &identity.public_key_hex,
            &request.profile_dht_key,
            32,
        );

        // Read-append-write
        let existing = self
            .io
            .read_record(&req_inbox_key, subkey, true)
            .await?
            .unwrap_or_default();

        let mut entries: Vec<FriendRequestEntry> = if existing.is_empty() || existing == b"[]" {
            Vec::new()
        } else {
            FriendRequestEntry::parse_inbox_data(&existing).unwrap_or_default()
        };
        entries.retain(|e| e.sender_public_key != response.sender_public_key);
        entries.push(response);

        let bytes = serde_json::to_vec(&entries)
            .map_err(|e| ChatError::Serialization(format!("acceptance entry: {e}")))?;
        self.io
            .write_record(&req_inbox_key, subkey, &bytes, Some(&kp_bytes), crate::io::Confirm::Accepted)
            .await?;

        tracing::info!("acceptance written to requester's friend inbox");
        Ok(())
    }
}
