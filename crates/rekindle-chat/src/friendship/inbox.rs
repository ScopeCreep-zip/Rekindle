//! Friend inbox scan coordinator — coalesces 3-tier triggers into single scans.
//!
//! Tier 1: DHT watch fires on friend_inbox_key → on_record_change → trigger
//! Tier 2: FriendRequestAck app_message → on_message → trigger
//! Tier 3: 60s poll sweep detects sequence change → trigger
//!
//! All three call `trigger()` which sends to an mpsc channel. The coordinator
//! task drains the channel, enforces a 30-second cooldown, and runs one scan.

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use rekindle_identity::IdentityRoot;
use rekindle_storage::VaultStore;
use rekindle_storage::keys::labels;
use rekindle_types::dht_types::{
    FriendRequestEntry, FriendRequestStatus, FRIEND_INBOX_SUBKEY_COUNT,
};
use rekindle_types::dm_store::{DmInvitePending, DmParticipant, DmStore};
use rekindle_types::session_types::{DmSmplPeer, PendingFriendRequest, SessionMeta};
use rekindle_types::subscription_events::{SubscriptionEvent, FriendEvent};

use crate::crypto::sessions::SessionCache;
use crate::events::pipeline::EventPipeline;
use crate::events::registry::{WatchKind, WatchRegistry};
use crate::io::PlatformIO;
const SCAN_COOLDOWN_SECS: u64 = 30;

/// Coordinates inbox scans from multiple trigger sources.
pub struct InboxScanCoordinator {
    trigger_tx: mpsc::Sender<()>,
}

impl InboxScanCoordinator {
    /// Spawn the coordinator background task.
    pub fn spawn(
        io: Arc<PlatformIO>,
        vault: Arc<VaultStore>,
        session_meta: Arc<RwLock<SessionMeta>>,
        session_cache: Arc<SessionCache>,
        watches: Arc<WatchRegistry>,
        pipeline: Arc<EventPipeline>,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<()>(4);

        tokio::spawn(async move {
            let cooldown = std::time::Duration::from_secs(SCAN_COOLDOWN_SECS);
            let mut last_scan = std::time::Instant::now()
                .checked_sub(cooldown)
                .unwrap_or_else(std::time::Instant::now);

            while rx.recv().await.is_some() {
                if last_scan.elapsed() < cooldown {
                    debug!(
                        remaining_secs = (cooldown - last_scan.elapsed()).as_secs(),
                        "inbox scan coalesced"
                    );
                    while rx.try_recv().is_ok() {}
                    continue;
                }

                let inbox_key = {
                    let meta = session_meta.read();
                    meta.identity
                        .as_ref()
                        .map(|id| id.friend_inbox_key.clone())
                        .unwrap_or_default()
                };
                if inbox_key.is_empty() {
                    continue;
                }

                let acceptances = scan_inbox(
                    &io,
                    &vault,
                    &session_meta,
                    &session_cache,
                    &pipeline,
                    &inbox_key,
                ).await;
                last_scan = std::time::Instant::now();

                // Set up DM watches for discovered acceptances
                for acceptance in &acceptances {
                    match io.open_and_watch(
                        &acceptance.dm_log_key, &[0],
                        WatchKind::DmLog { peer_key: acceptance.peer_key.clone() },
                        &watches,
                    ).await {
                        Ok(()) => {
                            info!(
                                peer = &acceptance.peer_key[..16.min(acceptance.peer_key.len())],
                                "DM watch established (acceptance via inbox scan)"
                            );
                        }
                        Err(e) => {
                            warn!(
                                peer = &acceptance.peer_key[..16.min(acceptance.peer_key.len())],
                                error = %e,
                                "DM watch failed (acceptance via inbox scan)"
                            );
                        }
                    }

                    // SMPL DM watch
                    if let Some(ref rk) = acceptance.smpl_record_key {
                        let peer_subkey = match acceptance.smpl_my_subkey {
                            Some(0) => vec![1u32],
                            Some(1) => vec![0u32],
                            _ => vec![0, 1],
                        };
                        match io.open_and_watch(
                            rk, &peer_subkey,
                            WatchKind::DmSmpl { record_key: rk.clone() },
                            &watches,
                        ).await {
                            Ok(()) => {
                                info!(
                                    peer = &acceptance.peer_key[..16.min(acceptance.peer_key.len())],
                                    record_key = &rk[..12.min(rk.len())],
                                    "SMPL DM watch established (inbox scan)"
                                );
                            }
                            Err(e) => {
                                warn!(
                                    peer = &acceptance.peer_key[..16.min(acceptance.peer_key.len())],
                                    error = %e,
                                    "SMPL DM watch failed (inbox scan)"
                                );
                            }
                        }
                    }
                }

                while rx.try_recv().is_ok() {}
            }
        });

        Self { trigger_tx: tx }
    }

    /// Non-blocking trigger. Dropped if channel is full (coalesced).
    pub fn trigger(&self) {
        let _ = self.trigger_tx.try_send(());
    }

    /// Clone the trigger sender for wiring into services that need to
    /// trigger inbox scans without holding a reference to the coordinator.
    pub fn trigger_sender(&self) -> mpsc::Sender<()> {
        self.trigger_tx.clone()
    }
}

/// A discovered acceptance from the inbox scan.
pub struct DiscoveredAcceptance {
    pub peer_key: String,
    pub dm_log_key: String,
    /// SMPL DM record key from the Accepted entry.
    pub smpl_record_key: Option<String>,
    /// Our subkey in the SMPL record.
    pub smpl_my_subkey: Option<u32>,
}

/// Scan the friend inbox for new requests and acceptances.
async fn scan_inbox(
    io: &Arc<PlatformIO>,
    vault: &Arc<VaultStore>,
    session_meta: &Arc<RwLock<SessionMeta>>,
    session_cache: &Arc<SessionCache>,
    pipeline: &Arc<EventPipeline>,
    inbox_key: &str,
) -> Vec<DiscoveredAcceptance> {
    // Load signing seed ONCE, derive identity root and X25519 seed for all
    // respond_to_acceptance calls in this scan.
    let signing_seed: Option<[u8; 32]> = vault
        .require_key(labels::SIGNING_KEY)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok());

    // Derive our identity root from the signing seed
    let our_identity_root: Option<IdentityRoot> = signing_seed.and_then(|seed| {
        let kp = rekindle_identity::SigningKeypair::from_seed(&seed).ok()?;
        IdentityRoot::from_bytes(kp.public_key_bytes()).ok()
    });

    // Derive X25519 identity seed (G2 derivation)
    let our_x25519_seed: Option<[u8; 32]> = signing_seed.map(|seed| {
        rekindle_identity::x25519_seed_from(&seed)
    });
    let start = std::time::Instant::now();
    info!(inbox_key = &inbox_key[..20.min(inbox_key.len())], "inbox scan starting");

    let inbox_record = match io.open_record(inbox_key, None).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "inbox open failed");
            return Vec::new();
        }
    };

    // Inspect to find populated subkeys
    let all_subkeys: Vec<u32> = (0..FRIEND_INBOX_SUBKEY_COUNT).collect();
    let inspect = match io.inspect_record(&inbox_record, &all_subkeys).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "inbox inspect failed");
            return Vec::new();
        }
    };

    // Use network_seqs to discover subkeys that REMOTE nodes have.
    // Alice's friend request may exist on the network but not in
    // Bob's local cache yet. Fall back to local_seqs if the network
    // returned nothing (offline or no remote holders responded).
    let check_seqs = if inspect.network_seqs.iter().any(|s| s.is_some()) {
        &inspect.network_seqs
    } else {
        &inspect.local_seqs
    };

    let populated: Vec<u32> = check_seqs
        .iter()
        .enumerate()
        .filter(|(_, seq)| seq.is_some())
        .map(|(i, _)| u32::try_from(i).unwrap_or(0))
        .collect();

    if populated.is_empty() {
        info!(elapsed_ms = start.elapsed().as_millis(), "inbox scan: empty");
        return Vec::new();
    }

    let mut new_acceptances = Vec::new();
    let mut found_new = 0u32;

    for subkey in &populated {
        let data = match io.read_record(&inbox_record, *subkey, true).await {
            Ok(Some(d)) if !d.is_empty() && d != b"[]" => d,
            _ => continue,
        };

        let entries = match FriendRequestEntry::parse_inbox_data(&data) {
            Ok(e) => e,
            Err(e) => {
                warn!(subkey, error = %e, "inbox parse failed");
                continue;
            }
        };

        for entry in entries {
            // Signature verification
            let sig_valid = verify_entry_signature(&entry);
            if !sig_valid {
                warn!(
                    from = %entry.display_name,
                    sender = entry.sender_public_key.display_short(),
                    "rejecting entry — invalid signature"
                );
                continue;
            }

            // Process Accepted entries
            if let FriendRequestStatus::Accepted {
                ref responder_outbound_log_key,
                ref pqxdh_init_message,
                ref dm_smpl_record_key,
                ref dm_smpl_slot_seed_hex,
                ..
            } = entry.status
            {
                // Skip if already friends (mutual accept guard).
                let already = {
                    let meta = session_meta.read();
                    meta.dm_peers.contains_key(&entry.sender_public_key.to_hex())
                };
                if already {
                    continue;
                }

                // ── PQXDH respond BEFORE dm_peers insertion ────────────
                // If respond fails, we do NOT update dm_peers. The TUI won't
                // show the friend. Next scan retries from scratch.
                if let (Some(ref seed), Some(ref root), Some(ref x_seed)) =
                    (&signing_seed, &our_identity_root, &our_x25519_seed)
                {
                    let _ = seed; // signing_seed used only for guard; root+x_seed do the work
                    if let Err(e) = crate::friendship::respond::respond_to_acceptance(
                        vault,
                        session_cache,
                        &entry.sender_public_key.to_hex(),
                        &entry.profile_dht_key,
                        pqxdh_init_message,
                        root,
                        x_seed,
                    ).await {
                        warn!(
                            error = %e,
                            peer = entry.sender_public_key.display_short(),
                            "PQXDH respond failed — will retry next scan"
                        );
                        // Do NOT remove from pending, do NOT update dm_peers.
                        continue;
                    }
                } else {
                    warn!("signing seed unavailable — cannot complete PQXDH respond");
                    continue;
                }

                // ── State update (only after successful respond) ───────
                {
                    let mut meta = session_meta.write();
                    meta.friend_display_names
                        .insert(entry.sender_public_key.to_hex(), entry.display_name.clone());
                    meta.remove_pending_friend_request(&entry.sender_public_key.to_hex());
                    let our_outbound = meta
                        .pending_outbound_logs
                        .remove(&entry.profile_dht_key)
                        .unwrap_or_default();
                    let peer_log = meta
                        .dm_peers
                        .entry(entry.sender_public_key.to_hex())
                        .or_insert_with(|| rekindle_types::session_types::DmPeerLog {
                            outbound_log_key: String::new(),
                            inbound_log_key: String::new(),
                        });
                    if !our_outbound.is_empty() {
                        peer_log.outbound_log_key = our_outbound;
                    }
                    peer_log.inbound_log_key.clone_from(responder_outbound_log_key);
                }

                let _ = vault.store_friend_name(&entry.sender_public_key.to_hex(), &entry.display_name);
                let _ = vault.take_pending_outbound(&entry.profile_dht_key);

                // Discover and cache the accepted peer's route blob
                let _ = io.discover_peer_route(
                    &entry.sender_public_key.to_hex(),
                    &entry.profile_dht_key,
                    None,
                ).await;

                // ── Emit event AFTER state is persisted ────────────────
                pipeline.process(SubscriptionEvent::Friend(FriendEvent::Accepted {
                    peer_key: entry.sender_public_key.to_hex(),
                    dm_log_key: responder_outbound_log_key.clone(),
                }));

                // ── SMPL DM record from Accepted entry ─────────────────
                let mut smpl_rk_for_acceptance = None;
                let mut smpl_subkey_for_acceptance = None;

                if let (Some(ref rk), Some(ref seed_hex)) = (dm_smpl_record_key, dm_smpl_slot_seed_hex) {
                    // Acceptor created the SMPL record — discover and use it
                    if let Ok(seed_bytes) = hex::decode(seed_hex) {
                        if seed_bytes.len() == 32 {
                            let peer_hex = entry.sender_public_key.to_hex();
                            let our_hex = our_identity_root
                                .map(|r| r.to_hex())
                                .unwrap_or_default();

                            if our_hex.is_empty() {
                                warn!("inbox scan: identity root unavailable — skipping SMPL DM setup");
                            } else {
                                // Acceptor always gets subkey 0 (accept.rs:208).
                                // Requester always gets subkey 1.
                                let my_sub = 1u32;

                                info!(
                                    record_key = &rk[..20.min(rk.len())],
                                    peer = entry.sender_public_key.display_short(),
                                    my_subkey = my_sub,
                                    "inbox scan: discovered SMPL DM record from Accepted entry"
                                );

                                match vault.dm_persist_invite(DmInvitePending {
                                    record_key: rk.clone(),
                                    is_group: false,
                                    initiator_public_key: peer_hex.clone(),
                                    initiator_pseudonym: entry.display_name.clone(),
                                    my_subkey: my_sub,
                                    participants: vec![
                                        DmParticipant {
                                            pseudonym: if my_sub == 0 { our_hex.clone() } else { entry.display_name.clone() },
                                            subkey: 0,
                                            public_key: if my_sub == 0 { our_hex.clone() } else { peer_hex.clone() },
                                        },
                                        DmParticipant {
                                            pseudonym: if my_sub == 1 { our_hex.clone() } else { entry.display_name.clone() },
                                            subkey: 1,
                                            public_key: if my_sub == 1 { our_hex.clone() } else { peer_hex.clone() },
                                        },
                                    ],
                                    mek_generation: 0,
                                    slot_seed_hex: seed_hex.clone(),
                                    wrapped_mek_blob: None,
                                }) {
                                    Ok(()) => {
                                        {
                                            let mut meta = session_meta.write();
                                            meta.dm_smpl_peers.insert(
                                                peer_hex,
                                                DmSmplPeer {
                                                    record_key: rk.clone(),
                                                    is_group: false,
                                                },
                                            );
                                        }

                                        smpl_rk_for_acceptance = Some(rk.clone());
                                        smpl_subkey_for_acceptance = Some(my_sub);
                                    }
                                    Err(e) => {
                                        warn!(
                                            error = %e,
                                            record_key = &rk[..12.min(rk.len())],
                                            "inbox scan: vault persist failed — SMPL record not locally tracked"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                found_new += 1;
                new_acceptances.push(DiscoveredAcceptance {
                    peer_key: entry.sender_public_key.to_hex(),
                    dm_log_key: responder_outbound_log_key.clone(),
                    smpl_record_key: smpl_rk_for_acceptance,
                    smpl_my_subkey: smpl_subkey_for_acceptance,
                });
                info!(
                    from = %entry.display_name,
                    sender = entry.sender_public_key.display_short(),
                    "ACCEPTANCE confirmed — PQXDH session established"
                );
                continue;
            }

            // Process Rejected entries
            if matches!(entry.status, FriendRequestStatus::Rejected { .. }) {
                let was_pending = {
                    let meta = session_meta.read();
                    meta.pending_outbound_logs.contains_key(&entry.profile_dht_key)
                };
                if was_pending {
                    {
                        let mut meta = session_meta.write();
                        meta.pending_outbound_logs.remove(&entry.profile_dht_key);
                    }
                    pipeline.process(SubscriptionEvent::Friend(FriendEvent::Rejected {
                        peer_key: entry.sender_public_key.to_hex(),
                    }));
                    info!(
                        from = %entry.display_name,
                        sender = entry.sender_public_key.display_short(),
                        "REJECTION discovered"
                    );
                }
                continue;
            }

            // Skip non-pending (unknown status variants)
            if !matches!(entry.status, FriendRequestStatus::Pending) {
                continue;
            }

            // Skip already friends
            let already_friends = {
                let meta = session_meta.read();
                meta.dm_peers.contains_key(&entry.sender_public_key.to_hex())
            };
            if already_friends {
                continue;
            }

            // Skip already known pending
            let already_known = {
                let meta = session_meta.read();
                meta.pending_friend_requests
                    .iter()
                    .any(|r| r.sender_public_key == entry.sender_public_key.to_hex())
            };
            if already_known {
                continue;
            }

            // Discover and cache the requester's route blob for direct messaging
            let _ = io.open_record(&entry.profile_dht_key, None).await.map(|_| ());
            let _ = io.discover_peer_route(
                &entry.sender_public_key.to_hex(),
                &entry.profile_dht_key,
                None,
            ).await;

            // Persist new pending request
            let pending = PendingFriendRequest {
                sender_public_key: entry.sender_public_key.to_hex(),
                display_name: entry.display_name.clone(),
                message: entry.message.clone(),
                profile_dht_key: entry.profile_dht_key.clone(),
                mailbox_dht_key: entry.mailbox_dht_key.clone(),
                prekey_bundle: entry.prekey_bundle.clone(),
                dm_log_key: entry.dm_log_key.clone(),
                dm_log_keypair_hex: entry.dm_log_keypair_hex.clone(),
                received_at: entry.sent_at,
            };
            {
                let mut meta = session_meta.write();
                meta.pending_friend_requests.push(pending);
            }

            // Emit event AFTER state is persisted.
            pipeline.process(SubscriptionEvent::Friend(FriendEvent::RequestReceived {
                from_key: entry.sender_public_key.to_hex(),
                display_name: entry.display_name.clone(),
                message: entry.message.clone(),
            }));

            found_new += 1;
            info!(
                from = %entry.display_name,
                sender = entry.sender_public_key.display_short(),
                "NEW request discovered"
            );
        }
    }

    info!(
        elapsed_ms = start.elapsed().as_millis(),
        new = found_new,
        acceptances = new_acceptances.len(),
        "inbox scan complete"
    );

    new_acceptances
}

/// Verify Ed25519 signature on a FriendRequestEntry.
fn verify_entry_signature(entry: &FriendRequestEntry) -> bool {
    if entry.signature_hex.is_empty() {
        return false;
    }
    let Ok(sig_bytes) = hex::decode(&entry.signature_hex) else { return false };
    if sig_bytes.len() != 64 {
        return false;
    }
    let content = entry.signature_content();
    rekindle_identity::verify_ec_prekey(entry.sender_public_key.as_bytes(), &content, &sig_bytes).is_ok()
}
