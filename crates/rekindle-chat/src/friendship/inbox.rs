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

use rekindle_storage::VaultStore;
use rekindle_storage::keys::labels;
use rekindle_types::dht_types::{
    FriendRequestEntry, FriendRequestStatus, FRIEND_INBOX_SUBKEY_COUNT,
};
use rekindle_types::session_types::{PendingFriendRequest, SessionMeta};
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
                    match io.watch_record(&acceptance.dm_log_key, &[0]).await {
                        Ok(token) => {
                            watches.register(
                                &acceptance.dm_log_key,
                                WatchKind::DmLog {
                                    peer_key: acceptance.peer_key.clone(),
                                },
                                token,
                            );
                            info!(
                                peer = &acceptance.peer_key[..16.min(acceptance.peer_key.len())],
                                "DM watch established (acceptance via inbox scan)"
                            );
                        }
                        Err(e) => {
                            warn!(
                                peer = &acceptance.peer_key[..16.min(acceptance.peer_key.len())],
                                error = %e,
                                "failed to establish DM watch"
                            );
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
    // Load signing seed ONCE for all respond_to_acceptance calls in this scan.
    let signing_seed: Option<[u8; 32]> = vault
        .require_key(labels::SIGNING_KEY)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok());
    let start = std::time::Instant::now();
    info!(inbox_key = &inbox_key[..20.min(inbox_key.len())], "inbox scan starting");

    let _ = io.open_record(inbox_key, None).await;

    // Inspect to find populated subkeys
    let all_subkeys: Vec<u32> = (0..FRIEND_INBOX_SUBKEY_COUNT).collect();
    let seqs = match io.inspect_record(inbox_key, &all_subkeys).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "inbox inspect failed");
            return Vec::new();
        }
    };

    let populated: Vec<u32> = seqs
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
        let data = match io.read_record(inbox_key, *subkey, true).await {
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
                    sender = &entry.sender_public_key[..16.min(entry.sender_public_key.len())],
                    "rejecting entry — invalid signature"
                );
                continue;
            }

            // Process Accepted entries
            if let FriendRequestStatus::Accepted {
                ref responder_outbound_log_key,
                ref pqxdh_init_message,
                ..
            } = entry.status
            {
                // Skip if already friends (mutual accept guard).
                let already = {
                    let meta = session_meta.read();
                    meta.dm_peers.contains_key(&entry.sender_public_key)
                };
                if already {
                    continue;
                }

                // ── PQXDH respond BEFORE dm_peers insertion ────────────
                // If respond fails, we do NOT update dm_peers. The TUI won't
                // show the friend. Next scan retries from scratch.
                if let Some(ref seed) = signing_seed {
                    if let Err(e) = crate::friendship::respond::respond_to_acceptance(
                        vault,
                        session_cache,
                        &entry.sender_public_key,
                        pqxdh_init_message,
                        seed,
                    ).await {
                        warn!(
                            error = %e,
                            peer = &entry.sender_public_key[..16.min(entry.sender_public_key.len())],
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
                        .insert(entry.sender_public_key.clone(), entry.display_name.clone());
                    meta.remove_pending_friend_request(&entry.sender_public_key);
                    let our_outbound = meta
                        .pending_outbound_logs
                        .remove(&entry.profile_dht_key)
                        .unwrap_or_default();
                    let peer_log = meta
                        .dm_peers
                        .entry(entry.sender_public_key.clone())
                        .or_insert_with(|| rekindle_types::session_types::DmPeerLog {
                            outbound_log_key: String::new(),
                            inbound_log_key: String::new(),
                        });
                    if !our_outbound.is_empty() {
                        peer_log.outbound_log_key = our_outbound;
                    }
                    peer_log.inbound_log_key.clone_from(responder_outbound_log_key);
                }

                let _ = vault.store_friend_name(&entry.sender_public_key, &entry.display_name);
                let _ = vault.take_pending_outbound(&entry.profile_dht_key);

                // ── Emit event AFTER state is persisted ────────────────
                pipeline.process(SubscriptionEvent::Friend(FriendEvent::Accepted {
                    peer_key: entry.sender_public_key.clone(),
                    dm_log_key: responder_outbound_log_key.clone(),
                }));

                found_new += 1;
                new_acceptances.push(DiscoveredAcceptance {
                    peer_key: entry.sender_public_key.clone(),
                    dm_log_key: responder_outbound_log_key.clone(),
                });
                info!(
                    from = %entry.display_name,
                    sender = &entry.sender_public_key[..16.min(entry.sender_public_key.len())],
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
                        peer_key: entry.sender_public_key.clone(),
                    }));
                    info!(
                        from = %entry.display_name,
                        sender = &entry.sender_public_key[..16.min(entry.sender_public_key.len())],
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
                meta.dm_peers.contains_key(&entry.sender_public_key)
            };
            if already_friends {
                continue;
            }

            // Skip already known pending
            let already_known = {
                let meta = session_meta.read();
                meta.pending_friend_requests
                    .iter()
                    .any(|r| r.sender_public_key == entry.sender_public_key)
            };
            if already_known {
                continue;
            }

            // Persist new pending request
            let pending = PendingFriendRequest {
                sender_public_key: entry.sender_public_key.clone(),
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
                from_key: entry.sender_public_key.clone(),
                display_name: entry.display_name.clone(),
                message: entry.message.clone(),
            }));

            found_new += 1;
            info!(
                from = %entry.display_name,
                sender = &entry.sender_public_key[..16.min(entry.sender_public_key.len())],
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
    let Ok(pub_bytes) = hex::decode(&entry.sender_public_key) else { return false };
    if sig_bytes.len() != 64 || pub_bytes.len() != 32 {
        return false;
    }
    let mut pub_arr = [0u8; 32];
    pub_arr.copy_from_slice(&pub_bytes);
    let content = entry.signature_content();
    rekindle_ratchet::crypto::sign::verify_ec_prekey(&pub_arr, &content, &sig_bytes).is_ok()
}
