//! Accept a pending friend request — PQXDH handshake, DhtLog setup, inbox write.

use aws_lc_rs::rand::SecureRandom;
use rekindle_types::dm_store::{DmInvitePending, DmParticipant, DmStore};
use rekindle_types::transport::RecordSchema;
use rekindle_types::dht_types::{
    FriendEntry, FriendRequestEntry, FriendRequestStatus,
    PROFILE_SUBKEY_FRIEND_INBOX_KEY, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
};
use rekindle_types::session_types::{DmPeerLog, DmSmplPeer};
use rekindle_storage::keys::labels;

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
        // Retry-with-backoff: scan inbox until request is found or 30s deadline.
        self.ensure_request_discovered(peer_pubkey).await;

        let kp = self.io.signing_keypair()?;
        let our_x25519_seed = self.io.x25519_identity_seed()?;
        let our_root = self.io.identity_root()?;
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
            meta.pending_request_by_key_hex(peer_pubkey)
                .cloned()
                .ok_or_else(|| ChatError::RequestNotFound {
                    peer_key: peer_pubkey.to_string(),
                })?
        };

        // Check for mutual case: they already accepted OUR request.
        let mutual_result = self
            .check_mutual_acceptance(peer_pubkey, &identity, &our_root, &our_x25519_seed)
            .await?;
        if let Some(accepted) = mutual_result {
            return Ok(accepted);
        }

        // PQXDH initiate using their prekey bundle
        let our_ed_pub = kp.public_key_bytes();

        // Deserialize the peer's PreKeyBundle from the pending request
        let peer_bundle: rekindle_ratchet::pqxdh::bundle::PreKeyBundle =
            serde_json::from_slice(&request.prekey_bundle)
                .map_err(|e| ChatError::Deserialization(format!("peer prekey bundle: {e}")))?;

        // Validate the bundle (signature verification, length checks, freshness)
        let now_secs = timestamp_secs();
        rekindle_ratchet::pqxdh::verify::validate_bundle(&peer_bundle, now_secs)?;

        // Run PQXDH initiator handshake
        let pqxdh_result = rekindle_ratchet::pqxdh::initiate(
            &our_x25519_seed,
            &our_ed_pub,
            &peer_bundle,
        )?;

        let ec_state = pqxdh_result.ec_state;

        // Serialize the PQXDH init message for the Accepted entry
        let pqxdh_init_message_bytes = serde_json::to_vec(&pqxdh_result.init_message)
            .map_err(|e| ChatError::Serialization(format!("pqxdh init message: {e}")))?;

        // Session ID via identity crate's session_anchor — typed roots, lexicographic ordering
        let peer_root = rekindle_identity::IdentityRoot::from_hex(peer_pubkey)
            .map_err(|e| ChatError::Internal(format!("peer root: {e}")))?;
        let session_anchor = rekindle_identity::session_anchor(&our_root, &peer_root)
            .map_err(|e| ChatError::Internal(format!("session anchor: {e}")))?;
        let session_id_bytes: [u8; 32] = *session_anchor.as_bytes();

        let trust_level = rekindle_ratchet::session::TrustLevel::TrustOnFirstUse { full_fs: false };
        let session = rekindle_ratchet::session::TripleRatchetSession::new(
            session_id_bytes,
            rekindle_ratchet::session::Direction::Initiator,
            ec_state,
            trust_level,
        );

        // Persist session to vault THEN insert into cache
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
            "friendship::accept: persisting session to vault"
        );

        self.vault.store_session(
            &session_id_bytes,
            peer_pubkey,
            direction_byte,
            &cbor,
            session.spqr_active,
            0,
        )?;

        tracing::debug!(
            peer = &peer_pubkey[..16.min(peer_pubkey.len())],
            "friendship::accept: session persisted — inserting into cache"
        );

        self.session_cache.insert(session_id_bytes, session).await;

        // Create our outbound DhtLog
        let (outbound_record, outbound_keypair) = self
            .io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let outbound_key = outbound_record.key().to_string();
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

        // ── SMPL DM record with lexicographic tiebreak ─────────────
        let our_hex = identity.public_key.to_hex();

        let (dm_smpl_record_key, dm_smpl_slot_seed_hex) = {
            let mut slot_seed = [0u8; 32];
            aws_lc_rs::rand::SystemRandom::new()
                .fill(&mut slot_seed)
                .map_err(|e| ChatError::Internal(format!("smpl slot seed rng: {e}")))?;

            let our_slot = rekindle_identity::derive_slot_keypair(&slot_seed, 0)
                .map_err(|e| ChatError::Internal(format!("smpl slot 0: {e}")))?;
            let peer_slot = rekindle_identity::derive_slot_keypair(&slot_seed, 1)
                .map_err(|e| ChatError::Internal(format!("smpl slot 1: {e}")))?;

            let schema = RecordSchema::MultiWriter {
                owner_subkeys: 0,
                member_subkeys: 1,
                member_keys: vec![our_slot.public_key_bytes(), peer_slot.public_key_bytes()],
            };
            let (smpl_record, _) = self.io.create_record(schema).await?;
            let record_key = smpl_record.key().to_string();

            tracing::info!(
                record_key = &record_key[..20.min(record_key.len())],
                peer = &peer_pubkey[..16.min(peer_pubkey.len())],
                "friendship::accept: SMPL DM record created"
            );

            // Persist to vault — gate ALL downstream state on success
            match self.vault.dm_persist_invite(DmInvitePending {
                record_key: record_key.clone(),
                is_group: false,
                initiator_public_key: our_hex.clone(),
                initiator_pseudonym: identity.display_name.clone(),
                my_subkey: 0,
                participants: vec![
                    DmParticipant {
                        pseudonym: identity.display_name.clone(),
                        subkey: 0,
                        public_key: our_hex.clone(),
                    },
                    DmParticipant {
                        pseudonym: request.display_name.clone(),
                        subkey: 1,
                        public_key: peer_pubkey.to_string(),
                    },
                ],
                mek_generation: 0,
                slot_seed_hex: hex::encode(slot_seed),
                wrapped_mek_blob: None,
            }) {
                Ok(()) => {
                    {
                        let mut meta = self.session_meta.write();
                        meta.dm_smpl_peers.insert(
                            peer_pubkey.to_string(),
                            DmSmplPeer {
                                record_key: record_key.clone(),
                                is_group: false,
                            },
                        );
                    }

                    if let Err(e) = self.io.watch_and_register(
                        &smpl_record, &[1],
                        crate::events::registry::WatchKind::DmSmpl { record_key: record_key.clone() },
                        &self.watches,
                    ).await {
                        tracing::warn!(error = %e, "friendship::accept: SMPL DM watch failed");
                    }

                    (Some(record_key), Some(hex::encode(slot_seed)))
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        record_key = &record_key[..12.min(record_key.len())],
                        "friendship::accept: vault persist failed — SMPL record created but not locally tracked"
                    );
                    (None, None)
                }
            }
        };

        // Write Accepted entry to requester's inbox
        self.write_acceptance_to_inbox(
            &identity,
            &kp,
            &request,
            &outbound_key,
            &outbound_keypair_hex,
            &pqxdh_init_message_bytes,
            dm_smpl_record_key.as_deref(),
            dm_smpl_slot_seed_hex.as_deref(),
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

        // Persist friend display name
        self.vault.store_friend_name(peer_pubkey, &request.display_name)?;

        // Persist FriendEntry to the friend list DHT record
        let friend_entry = FriendEntry {
            public_key: rekindle_identity::IdentityRoot::from_hex(peer_pubkey)
                .map_err(|e| ChatError::Internal(format!("peer root: {e}")))?,
            nickname: None,
            group: None,
            added_at: timestamp_ms(),
            profile_dht_key: Some(request.profile_dht_key.clone()),
            dm_log_key: Some(outbound_key.clone()),
        };
        let fl_keypair = self.vault.load_key(
            rekindle_storage::keys::labels::FRIEND_LIST_KEYPAIR,
        )?;
        if let Some(ref kp_bytes) = fl_keypair {
            let fl_key = {
                let meta = self.session_meta.read();
                meta.identity.as_ref().map(|i| i.friend_list_dht_key.clone()).unwrap_or_default()
            };
            if !fl_key.is_empty() {
                let fl_record = self.io.open_record(&fl_key, None).await?;
                let existing = self.io.read_record(&fl_record, 0, false).await?.unwrap_or_default();
                let mut friend_list: rekindle_types::dht_types::FriendList =
                    if existing.is_empty() || existing == b"[]" {
                        rekindle_types::dht_types::FriendList::default()
                    } else {
                        serde_json::from_slice(&existing).unwrap_or_default()
                    };
                friend_list.friends.retain(|f| f.public_key.to_hex() != peer_pubkey);
                friend_list.friends.push(friend_entry);
                let bytes = serde_json::to_vec(&friend_list)
                    .map_err(|e| ChatError::Serialization(format!("friend list: {e}")))?;
                if let Err(e) = self.io.write_record(&fl_record, 0, &bytes, Some(kp_bytes), crate::io::Confirm::Accepted).await {
                    tracing::warn!(error = %e, "friend list DHT write failed — local vault copy is authoritative");
                }
            }
        }

        // Watch inbound log for DM receipt
        if let Err(e) = self.io.open_and_watch(
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
        our_root: &rekindle_identity::IdentityRoot,
        our_x25519_seed: &[u8; 32],
    ) -> Result<Option<FriendAccepted>, ChatError> {
        let target_subkey = blake3_hash_mod(
            peer_pubkey,
            &identity.profile_dht_key,
            32,
        );

        let inbox_record = self.io
            .open_record(&identity.friend_inbox_key, None)
            .await?;

        let data = self
            .io
            .read_record(&inbox_record, target_subkey, true)
            .await?;

        let Some(data) = data else { return Ok(None) };
        if data.is_empty() || data == b"[]" {
            return Ok(None);
        }

        let entries = FriendRequestEntry::parse_inbox_data(&data).unwrap_or_default();

        let accepted_entry = entries.iter().find(|e| {
            e.sender_public_key.to_hex() == peer_pubkey
                && matches!(e.status, FriendRequestStatus::Accepted { .. })
        });

        let Some(entry) = accepted_entry else {
            return Ok(None);
        };

        let FriendRequestStatus::Accepted {
            ref responder_outbound_log_key,
            ref pqxdh_init_message,
            ref dm_smpl_record_key,
            ref dm_smpl_slot_seed_hex,
            ..
        } = entry.status
        else {
            return Ok(None);
        };

        tracing::info!(
            peer = &peer_pubkey[..16.min(peer_pubkey.len())],
            "mutual acceptance detected — they already accepted our request"
        );

        // Complete PQXDH handshake
        if !pqxdh_init_message.is_empty() {
            if let Err(e) = crate::friendship::respond::respond_to_acceptance(
                &self.vault,
                &self.session_cache,
                peer_pubkey,
                &entry.profile_dht_key,
                pqxdh_init_message,
                our_root,
                our_x25519_seed,
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
                .pending_request_by_key_hex(peer_pubkey)
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

        // ── SMPL DM record from mutual acceptance ──────────────────
        if let (Some(ref rk), Some(ref seed_hex)) = (dm_smpl_record_key, dm_smpl_slot_seed_hex) {
            tracing::info!(
                record_key = &rk[..20.min(rk.len())],
                "friendship::mutual: reading SMPL record from peer's Accepted entry"
            );

            if let Ok(seed_bytes) = hex::decode(seed_hex) {
                if seed_bytes.len() == 32 {
                    match self.vault.dm_persist_invite(DmInvitePending {
                        record_key: rk.clone(),
                        is_group: false,
                        initiator_public_key: entry.sender_public_key.to_hex(),
                        initiator_pseudonym: entry.display_name.clone(),
                        my_subkey: 1,
                        participants: vec![
                            DmParticipant {
                                pseudonym: entry.display_name.clone(),
                                subkey: 0,
                                public_key: entry.sender_public_key.to_hex(),
                            },
                            DmParticipant {
                                pseudonym: identity.display_name.clone(),
                                subkey: 1,
                                public_key: identity.public_key.to_hex(),
                            },
                        ],
                        mek_generation: 0,
                        slot_seed_hex: seed_hex.clone(),
                        wrapped_mek_blob: None,
                    }) {
                        Ok(()) => {
                            {
                                let mut meta = self.session_meta.write();
                                meta.dm_smpl_peers.insert(
                                    entry.sender_public_key.to_hex(),
                                    DmSmplPeer {
                                        record_key: rk.clone(),
                                        is_group: false,
                                    },
                                );
                            }

                            if let Err(e) = self.io.open_and_watch(
                                rk, &[0],
                                crate::events::registry::WatchKind::DmSmpl { record_key: rk.clone() },
                                &self.watches,
                            ).await {
                                tracing::warn!(error = %e, "friendship::mutual: SMPL DM watch failed");
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                record_key = &rk[..12.min(rk.len())],
                                "friendship::mutual: vault persist failed — SMPL record not locally tracked"
                            );
                        }
                    }
                }
            }
        }

        // Watch inbound log
        if let Err(e) = self.io.open_and_watch(
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
        signing_keypair: &rekindle_identity::SigningKeypair,
        request: &rekindle_types::session_types::PendingFriendRequest,
        outbound_key: &str,
        outbound_keypair_hex: &str,
        pqxdh_init_message_bytes: &[u8],
        dm_smpl_record_key: Option<&str>,
        dm_smpl_slot_seed_hex: Option<&str>,
    ) -> Result<(), ChatError> {
        // Read requester's inbox key from their profile
        let profile_record = self.io
            .open_record(&request.profile_dht_key, None)
            .await?;

        let req_inbox_key = self
            .io
            .read_record(&profile_record, PROFILE_SUBKEY_FRIEND_INBOX_KEY, true)
            .await?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        let req_inbox_kp = self
            .io
            .read_record(&profile_record, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR, true)
            .await?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();

        if req_inbox_key.is_empty() || req_inbox_kp.is_empty() {
            return Err(ChatError::InboxNotAvailable);
        }

        // Discover and cache the requester's route blob for direct messaging
        self.io.discover_peer_route(
            &request.sender_public_key,
            &request.profile_dht_key,
            None,
        ).await?;

        let kp_bytes = hex::decode(&req_inbox_kp)
            .map_err(|e| ChatError::Internal(format!("requester inbox kp: {e}")))?;

        let inbox_record = self.io
            .open_record(&req_inbox_key, Some(&kp_bytes))
            .await?;

        let mut response = FriendRequestEntry {
            sender_public_key: identity.public_key,
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
                dm_smpl_record_key: dm_smpl_record_key.map(String::from),
                dm_smpl_slot_seed_hex: dm_smpl_slot_seed_hex.map(String::from),
            },
        };

        let content = response.signature_content();
        let sig = signing_keypair.sign_ec_prekey(&content);
        response.signature_hex = hex::encode(sig);

        let subkey = blake3_hash_mod(
            &identity.public_key.to_hex(),
            &request.profile_dht_key,
            32,
        );

        // Read-append-write
        let existing = self
            .io
            .read_record(&inbox_record, subkey, true)
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
            .write_record(&inbox_record, subkey, &bytes, Some(&kp_bytes), crate::io::Confirm::Accepted)
            .await?;

        tracing::info!("acceptance written to requester's friend inbox");
        Ok(())
    }

    /// Scan inbox with exponential backoff until the pending request is
    /// found or 30s deadline elapses. Bridges DHT propagation delay.
    async fn ensure_request_discovered(&self, peer_pubkey: &str) {
        let deadline = std::time::Duration::from_secs(30);
        let start = std::time::Instant::now();
        let mut attempt = 0u32;
        loop {
            let found = self.session_meta.read()
                .pending_request_by_key_hex(peer_pubkey).is_some();
            if found {
                tracing::debug!(
                    peer = &peer_pubkey[..16.min(peer_pubkey.len())],
                    attempt,
                    elapsed_ms = start.elapsed().as_millis(),
                    "friendship::accept: request found after backoff"
                );
                return;
            }
            if start.elapsed() >= deadline {
                tracing::debug!(
                    peer = &peer_pubkey[..16.min(peer_pubkey.len())],
                    "friendship::accept: backoff deadline — request not found"
                );
                return;
            }
            self.trigger_inbox_scan();
            let wait = std::time::Duration::from_secs(2u64.saturating_pow(attempt).min(8));
            tokio::time::sleep(wait).await;
            attempt += 1;
        }
    }
}
