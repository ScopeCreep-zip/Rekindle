//! A friend's prekey record, read for a first contact (plan E2.4, B2/B3):
//! their device list from the profile, the record it names read from the
//! network, every signature checked under the identity we hold for them,
//! and one slot picked among those we have not used.

use rekindle_codec::capnp_codec::identity::{
    decode_opk_slot, decode_prekey_head, decode_prekey_record_list,
};
use rekindle_crypto::signal::prekey_record::{pick_slot, Pick, RecordView};
use rekindle_crypto::signal::PreKeyBundle;
use rekindle_crypto::CryptoError;
use rekindle_db::ratchet::PeerAddress;
use rekindle_protocol::dht::prekeys as record;
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
/// They published no prekeys, the record could not be read, or it is not
/// theirs.
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
    let pool = host.node()?.require_records()?;
    let list = record::read_device_list(pool.as_ref(), profile_key, true)
        .await
        .map_err(|e| unusable(format!("their profile could not be read ({e})")))?
        .ok_or_else(|| unusable("they have published no prekeys".to_string()))?;
    let list = decode_prekey_record_list(&list)
        .map_err(|e| unusable(format!("their device list: {e}")))?;
    // One device until E5.3 fans out per listed device.
    let device = list
        .devices
        .first()
        .ok_or_else(|| unusable("their device list is empty".to_string()))?;
    let read = record::read_record(pool.as_ref(), &device.record_key)
        .await
        .map_err(|e| unusable(format!("their prekey record could not be read ({e})")))?;
    let head = read
        .head
        .as_deref()
        .ok_or_else(|| unusable("their prekey record has no signed prekey yet".to_string()))
        .and_then(|bytes| {
            decode_prekey_head(bytes).map_err(|e| unusable(format!("their signed prekey: {e}")))
        })?;
    let slots = read
        .slots
        .iter()
        .filter_map(|(subkey, bytes)| decode_opk_slot(bytes).ok().map(|slot| (*subkey, slot)));
    let view = match RecordView::new(&identity, head, slots, host.clock().now_ms_i64()) {
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
    let address = PeerAddress::sole_device(identity);
    let listed: Vec<u32> = view.listed_opk_ids().collect();
    rekindle_db::prekeys::sweep_used_slots(&db, &owner, &address, listed)
        .await
        .map_err(|e| FriendError::store("sweep used prekey slots", e))?;
    let used = rekindle_db::prekeys::used_slots(&db, &owner, &address)
        .await
        .map_err(|e| FriendError::store("read used prekey slots", e))?;
    let pick = pick_slot(&view, &used, host.rng());
    if pick == Pick::LastResortOnly {
        tracing::info!(peer = %peer, "no unused one-time slot; initiating from the last-resort key");
    }
    view.bundle(&pick)
        .map_err(|e| unusable(format!("the picked slot: {e}")))
}
