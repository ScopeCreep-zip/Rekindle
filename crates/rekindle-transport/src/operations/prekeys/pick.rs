//! A friend's prekey record, read for a first contact (plan E2.4, B2/B3):
//! their device list from the profile, the record it names read from the
//! network, every signature checked under the identity we hold for them,
//! and one slot picked among those we have not used.
//!
//! The record is inspected once and only the head and the drawn slot are
//! read (R1-8): a slot drawn at random among those the inspect shows, and
//! another drawn only when that one does not decode, verify, or names a
//! one-time key we already used. Signal's bundle carries one one-time
//! prekey (PQXDH §3.3, Signal-Server `KeysManager`); two initiators that
//! draw the same slot meet B5's `PreKeyRejected` and the one retry.

use rekindle_codec::capnp_codec::identity::{
    decode_opk_slot, decode_prekey_head, decode_prekey_record_list, PrekeyRecordHead,
};
use rekindle_crypto::signal::prekey_record::{pick_slot, Pick, RecordView, UsedIds, OPK_STALE_MS};
use rekindle_crypto::signal::PreKeyBundle;
use rekindle_crypto::CryptoError;
use rekindle_db::ratchet::PeerAddress;
use rekindle_protocol::dht::prekeys::{self as record, PeerRecord};
use rekindle_protocol::error::ProtocolError;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use crate::operations::friend::{state, FriendError, FriendHost};

/// The bundle to open a session with `peer` (a hex identity) from: their
/// profile `profile_key`'s device list, the record it names, and the slot
/// B2 picks (the head alone when none is left).
///
/// A record not signed by `peer` — the identity we hold for them — opens
/// nothing and is never trusted: it raises `IdentityPinMismatch` for the
/// user (B3).
///
/// # Errors
/// `NotReady` while the bundle is not readable yet (no device list, the
/// record or its head not on the network or not reachable); otherwise
/// the record is not theirs or does not decode.
pub async fn pick_bundle(
    host: &dyn FriendHost,
    peer: &str,
    profile_key: &str,
) -> Result<PreKeyBundle, FriendError> {
    let identity = crate::operations::dm::recipient_key(peer)?;
    let unusable = |why: String| {
        FriendError::Conflict(format!(
            "A secure session with {peer} cannot be opened yet: {why}"
        ))
    };
    // Not readable yet (not named, not written, not reachable): the first
    // contact waits for it (live finding L-FR3).
    let waiting = |why: String| {
        FriendError::NotReady(format!(
            "A secure session with {peer} cannot be opened yet: {why}"
        ))
    };
    let pool = host.node()?.require_records()?;
    let list = record::read_device_list(pool.as_ref(), profile_key, true)
        .await
        .map_err(|e| waiting(format!("their profile could not be read ({e})")))?
        .ok_or_else(|| waiting("they have published no prekeys".to_string()))?;
    let list = decode_prekey_record_list(&list)
        .map_err(|e| unusable(format!("their device list: {e}")))?;
    // One device until E5.3 fans out per listed device.
    let device = list
        .devices
        .first()
        .ok_or_else(|| waiting("their device list is empty".to_string()))?;
    let read = PeerRecord::open(pool.as_ref(), &device.record_key)
        .await
        .map_err(|e| waiting(format!("their prekey record could not be read ({e})")))?;
    let bundle = bundle_from(host, peer, &identity, &read).await;
    read.close().await;
    bundle
}

/// The bundle from the opened record: the head checked, then a slot.
async fn bundle_from(
    host: &dyn FriendHost,
    peer: &str,
    identity: &[u8; 32],
    read: &PeerRecord<'_>,
) -> Result<PreKeyBundle, FriendError> {
    let unusable = |why: String| {
        FriendError::Conflict(format!(
            "A secure session with {peer} cannot be opened yet: {why}"
        ))
    };
    // Not readable yet (not named, not written, not reachable): the first
    // contact waits for it (live finding L-FR3).
    let waiting = |why: String| {
        FriendError::NotReady(format!(
            "A secure session with {peer} cannot be opened yet: {why}"
        ))
    };
    // The head and the first drawn slot are read together: one round trip
    // after the inspect.
    let mut untried = read.present_slots();
    let first = draw(host, &mut untried);
    let (head, first) = futures::join!(read.head(), async {
        match first {
            Some(subkey) => Some((subkey, read.slot(subkey).await)),
            None => None,
        }
    });
    let head = head
        .map_err(|e| waiting(format!("their signed prekey could not be read ({e})")))?
        .ok_or_else(|| waiting("their prekey record has no signed prekey yet".to_string()))
        .and_then(|bytes| {
            decode_prekey_head(&bytes).map_err(|e| unusable(format!("their signed prekey: {e}")))
        })?;
    let now_ms = host.clock().now_ms_i64();
    let view = match RecordView::new(identity, head.clone(), [], now_ms) {
        Ok(view) => view,
        Err(CryptoError::StalePrekeys {
            issued_at_ms,
            now_ms,
        }) => {
            let days = now_ms.saturating_sub(issued_at_ms).unsigned_abs() / 86_400_000;
            return Err(unusable(format!(
                "their signed prekey is {days} days out of date; their app has not refreshed it"
            )));
        }
        Err(e) => {
            tracing::warn!(peer = %peer, error = %e, "prekey record not signed by the pinned identity");
            // The trust alert stays on their row until a record verifies.
            state::update(host, peer, |f| f.identity_pin_mismatch = true);
            host.publish(SubscriptionEvent::Friend(
                FriendEvent::IdentityPinMismatch {
                    peer_key: peer.to_string(),
                    display_name: state::label(host, peer),
                },
            ));
            return Err(FriendError::Invalid(format!(
                "{peer}'s keys are not signed by the identity you know"
            )));
        }
    };

    // Their record verifies under the identity we hold: the alert is over.
    if state::friend(host, peer).is_some_and(|f| f.identity_pin_mismatch) {
        state::update(host, peer, |f| f.identity_pin_mismatch = false);
        host.publish(SubscriptionEvent::Friend(
            FriendEvent::IdentityPinRestored {
                peer_key: peer.to_string(),
            },
        ));
    }
    let (db, owner) = (host.db()?, host.local_party()?.owner());
    let address = PeerAddress::sole_device(*identity);
    // A slot we used was replaced within `OPK_STALE_MS` of its minting, so
    // a row older than that names a key the record no longer lists.
    rekindle_db::prekeys::sweep_used_slots(&db, &owner, &address, now_ms - OPK_STALE_MS)
        .await
        .map_err(|e| FriendError::store("sweep used prekey slots", e))?;
    let used = rekindle_db::prekeys::used_slots(&db, &owner, &address)
        .await
        .map_err(|e| FriendError::store("read used prekey slots", e))?;
    let check = SlotCheck {
        identity,
        head: &head,
        used: &used,
        now_ms,
    };
    let (picked, pick) = if let Some(found) = draw_slot(host, read, untried, first, &check).await {
        found
    } else {
        tracing::info!(peer = %peer, "no unused one-time slot; initiating from the last-resort key");
        (view, Pick::LastResortOnly)
    };
    picked
        .bundle(&pick)
        .map_err(|e| unusable(format!("the picked slot: {e}")))
}

/// What a drawn slot is checked against.
struct SlotCheck<'a> {
    identity: &'a [u8; 32],
    head: &'a PrekeyRecordHead,
    used: &'a UsedIds,
    now_ms: i64,
}

/// A slot drawn uniformly among `untried`, taken out of it.
fn draw(host: &dyn FriendHost, untried: &mut Vec<u32>) -> Option<u32> {
    if untried.is_empty() {
        return None;
    }
    let bound = u64::try_from(untried.len()).expect("at most SLOT_COUNT slots");
    let index = usize::try_from(host.rng().below(bound)).expect("an index below the count");
    Some(untried.swap_remove(index))
}

/// B2 over a record read slot by slot: `first` (drawn and read with the
/// head), then a slot drawn among `untried` at a time, read, and kept
/// when it decodes, verifies under the pinned identity and names a
/// one-time key not used; `None` when no slot does. A slot that fails to
/// read is skipped like an unusable one.
async fn draw_slot(
    host: &dyn FriendHost,
    read: &PeerRecord<'_>,
    mut untried: Vec<u32>,
    mut first: Option<(u32, Result<Option<Vec<u8>>, ProtocolError>)>,
    check: &SlotCheck<'_>,
) -> Option<(RecordView, Pick)> {
    loop {
        let (subkey, bytes) = if let Some(read_first) = first.take() {
            read_first
        } else {
            let subkey = draw(host, &mut untried)?;
            (subkey, read.slot(subkey).await)
        };
        let bytes = match bytes {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(e) => {
                tracing::debug!(subkey, error = %e, "prekey slot read failed; drawing another");
                continue;
            }
        };
        let Ok(slot) = decode_opk_slot(&bytes) else {
            tracing::debug!(subkey, "prekey slot does not decode; drawing another");
            continue;
        };
        let Ok(view) = RecordView::new(
            check.identity,
            check.head.clone(),
            [(subkey, slot)],
            check.now_ms,
        ) else {
            continue;
        };
        // One candidate: the pick keeps it unless we used its key.
        match pick_slot(&view, check.used, host.rng()) {
            pick @ Pick::Slot { .. } => return Some((view, pick)),
            Pick::LastResortOnly => {
                tracing::debug!(
                    subkey,
                    "prekey slot not usable or already used; drawing another"
                );
            }
        }
    }
}
