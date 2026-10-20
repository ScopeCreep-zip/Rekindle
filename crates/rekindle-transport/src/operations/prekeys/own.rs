//! Our prekey record (plan E2.4): publishing what the storage minted,
//! reconciling the record with our rows at unlock, the maintenance pass
//! (B1) and the status the CLI, the debug command and the status snapshot
//! read.

use rekindle_codec::capnp_codec::identity::{
    decode_prekey_head, encode_prekey_record_list, PrekeyDevice, PrekeyRecordList,
};
use rekindle_crypto::signal::prekey_record::{
    plan_maintenance, LOW_SLOT_WATERMARK, SIGNED_MAX_AGE_MS,
};
use rekindle_db::prekeys::Publish;
use rekindle_protocol::dht::prekeys as record;
use rekindle_types::dht_layout::prekeys::HEAD;
use rekindle_types::dto::identity::{PrekeyComponentLens, PrekeyStatus};
use rekindle_types::subscription_events::{CryptoEvent, SubscriptionEvent};

use super::stored;
use crate::operations::friend::{state, FriendError, FriendHost};

/// The profile's subkey 5: this device's entry naming `record_key` (one
/// entry until E5.3 binds devices).
///
/// # Errors
/// `device_id` is not 16 bytes of hex.
pub fn device_list(record_key: &str, device_id: &str) -> Result<Vec<u8>, FriendError> {
    let id: [u8; 16] = hex::decode(device_id)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| FriendError::Invalid(format!("device id {device_id} is not 16 bytes")))?;
    Ok(encode_prekey_record_list(&PrekeyRecordList {
        devices: vec![PrekeyDevice {
            device_id: id,
            record_key: record_key.to_string(),
        }],
    }))
}

/// Write `values` to our record, each durable (a miss is re-pushed until it
/// lands), and tell the frontends what was published: one event for the
/// whole call, naming the head and the slots written. A subkey whose write
/// fails does not stop the others (plan audit D-8); the next maintenance
/// tick's [`reconcile`] writes it again.
///
/// # Errors
/// Our record is not open yet, or a write failed outright: the error names
/// every subkey that failed (the event names what was written).
pub async fn publish(host: &dyn FriendHost, values: Vec<Publish>) -> Result<(), FriendError> {
    if values.is_empty() {
        return Ok(());
    }
    let lease = host
        .own_prekey_lease()
        .ok_or_else(|| FriendError::NotReady("our prekey record is not open yet".to_string()))?;
    let pool = host.node()?.require_records()?;
    let mut written = Written::default();
    let mut failed = Vec::new();
    for Publish { subkey, value } in values {
        let result = if subkey == HEAD {
            record::publish_head(pool.as_ref(), lease, value).await
        } else {
            record::publish_slot(pool.as_ref(), lease, subkey, value).await
        };
        match result {
            Ok(held) => {
                if held.missed() {
                    tracing::debug!(subkey, ?held, "prekey write held for re-push");
                }
                written.add(subkey);
            }
            Err(e) => {
                tracing::warn!(subkey, error = %e, "prekey subkey not published; continuing");
                failed.push(format!("subkey {subkey}: {e}"));
            }
        }
    }
    written.publish(host);
    if failed.is_empty() {
        Ok(())
    } else {
        Err(stored("publish prekeys", failed.join("; ")))
    }
}

/// What one [`publish`] call wrote.
#[derive(Debug, Default, PartialEq, Eq)]
struct Written {
    head: bool,
    slots: Vec<u32>,
}

impl Written {
    fn add(&mut self, subkey: u32) {
        if subkey == HEAD {
            self.head = true;
        } else {
            self.slots.push(subkey);
        }
    }

    /// The one `PrekeyRecordPublished` for what was written, if anything.
    fn into_event(mut self) -> Option<CryptoEvent> {
        if !self.head && self.slots.is_empty() {
            return None;
        }
        self.slots.sort_unstable();
        Some(CryptoEvent::PrekeyRecordPublished {
            head: self.head,
            slots: self.slots,
        })
    }

    fn publish(self, host: &dyn FriendHost) {
        if let Some(event) = self.into_event() {
            host.publish(SubscriptionEvent::Crypto(event));
        }
    }
}

/// The unlock's reconciliation: every value our rows say is published but
/// the record does not hold (a crash between the commit and the write, or
/// a record never written) is written again. Returns how many.
///
/// # Errors
/// Our record is not open, or it could not be read or written.
pub async fn reconcile(host: &dyn FriendHost) -> Result<usize, FriendError> {
    let lease = host
        .own_prekey_lease()
        .ok_or_else(|| FriendError::NotReady("our prekey record is not open yet".to_string()))?;
    let pool = host.node()?.require_records()?;
    let key = pool
        .key_of(lease)
        .ok_or_else(|| FriendError::NotReady("our prekey record was released".to_string()))?;
    let ours = rekindle_db::prekeys::published_values(&host.db()?, &host.local_party()?)
        .await
        .map_err(|e| stored("read our prekeys", e))?;
    let held = record::read_record(pool.as_ref(), &key)
        .await
        .map_err(|e| stored("read our prekey record", e))?;
    let differs = |p: &Publish| {
        if p.subkey == HEAD {
            held.head.as_deref() != Some(p.value.as_slice())
        } else {
            !held
                .slots
                .iter()
                .any(|(subkey, bytes)| *subkey == p.subkey && *bytes == p.value)
        }
    };
    let stale: Vec<Publish> = ours.into_iter().filter(differs).collect();
    let count = stale.len();
    publish(host, stale).await?;
    Ok(count)
}

/// One maintenance pass (B1): plan from our rows, mint and store what the
/// plan names in one transaction, publish it, sweep the archive, then
/// [`reconcile`] the record with our rows, so a write that failed in this
/// pass or an earlier one is retried each tick (plan audit D-8).
///
/// # Errors
/// The rows could not be read or written, or the record could not be
/// reconciled.
pub async fn maintain(host: &dyn FriendHost) -> Result<(), FriendError> {
    let values = plan_and_store(host).await?;
    if let Err(e) = publish(host, values).await {
        tracing::warn!(error = %e, "prekey publish incomplete; reconciling the record");
    }
    let rewritten = reconcile(host).await?;
    if rewritten > 0 {
        tracing::info!(rewritten, "prekey record reconciled with our rows");
    }
    Ok(())
}

/// [`maintain`] without the publish: what the pass minted and stored, the
/// head first, for a caller that publishes it in its own order (the
/// unlock writes the head and a few slots before the profile names the
/// record, the rest after it).
///
/// # Errors
/// The rows could not be read or written.
pub async fn plan_and_store(host: &dyn FriendHost) -> Result<Vec<Publish>, FriendError> {
    let (db, local) = (host.db()?, host.local_party()?);
    let owner = local.owner();
    let now = local.now_ms;
    let prekeys = rekindle_db::prekeys::local_prekeys(&db, &owner)
        .await
        .map_err(|e| stored("read our prekeys", e))?;
    let actions = plan_maintenance(now, &prekeys);
    let mut values = Vec::new();
    if !actions.is_empty() {
        tracing::info!(actions = actions.len(), "prekey maintenance");
        values = rekindle_db::prekeys::apply_maintenance(&db, &local, actions)
            .await
            .map_err(|e| stored("mint prekeys", e))?;
    }
    let swept = rekindle_db::prekeys::sweep_archive(&db, &owner, now)
        .await
        .map_err(|e| stored("sweep the prekey archive", e))?;
    tracing::debug!(?swept, "prekey archive swept");
    Ok(values)
}

/// This device's prekey record and its upkeep.
///
/// # Errors
/// No identity is unlocked, or the rows could not be read.
pub async fn status(host: &dyn FriendHost) -> Result<PrekeyStatus, FriendError> {
    let (db, local) = (host.db()?, host.local_party()?);
    let owner = state::owner(host)?;
    let row = rekindle_db::prekeys::status(&db, &local.owner(), local.now_ms)
        .await
        .map_err(|e| stored("read prekey status", e))?;
    let columns = state::db_call!(host, "read the identity row", move |conn| {
        rekindle_db::repo::identity::login_row(conn, &owner)
    })
    .await?
    .map(|(_, columns)| columns);
    let head = rekindle_db::prekeys::published_values(&db, &local)
        .await
        .map_err(|e| stored("read our prekeys", e))?
        .into_iter()
        .find(|p| p.subkey == HEAD)
        .and_then(|p| decode_prekey_head(&p.value).ok());
    let secs = |ms: i64| u64::try_from(ms / 1000).unwrap_or(0);
    Ok(PrekeyStatus {
        device_id: columns.as_ref().and_then(|c| c.device_id.clone()),
        record_key: columns.and_then(|c| c.prekeys),
        spk_id: row.map(|r| r.spk_id),
        spk_age_secs: row.map(|r| secs(r.spk_age_ms)),
        pq_lr_id: row.map(|r| r.pq_lr_id),
        lr_age_secs: row.map(|r| secs(r.lr_age_ms)),
        fresh_slots: row.map_or(0, |r| r.fresh_slots),
        consumed_slots: row.map_or(0, |r| r.consumed_slots),
        component_lens: head.map(|h| PrekeyComponentLens {
            spk: h.spk.len(),
            spk_sig: h.spk_sig.len(),
            pq_lr: h.pq_lr.len(),
            pq_lr_sig: h.pq_lr_sig.len(),
        }),
    })
}

/// Why the maintenance looks broken, if it does: no head yet, fewer fresh
/// slots than the watermark, or a head past the 14-day limit.
#[must_use]
pub fn prekey_problem(status: &PrekeyStatus) -> Option<String> {
    let (Some(spk), Some(lr)) = (status.spk_age_secs, status.lr_age_secs) else {
        return Some("no prekeys published yet".to_string());
    };
    if usize::try_from(status.fresh_slots).unwrap_or(usize::MAX) < LOW_SLOT_WATERMARK {
        return Some(format!("{}/100 fresh slots", status.fresh_slots));
    }
    let oldest = spk.max(lr);
    let limit = u64::try_from(SIGNED_MAX_AGE_MS / 1000).unwrap_or(u64::MAX);
    (oldest > limit).then(|| format!("signed prekeys {} days old", oldest / 86_400))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> PrekeyStatus {
        PrekeyStatus {
            device_id: None,
            record_key: None,
            spk_id: Some(1),
            spk_age_secs: Some(3_600),
            pq_lr_id: Some(2),
            lr_age_secs: Some(3_600),
            fresh_slots: 100,
            consumed_slots: 0,
            component_lens: None,
        }
    }

    #[test]
    fn the_problem_names_what_maintenance_failed_to_keep() {
        assert_eq!(prekey_problem(&healthy()), None);
        let low = PrekeyStatus {
            fresh_slots: 3,
            ..healthy()
        };
        assert_eq!(prekey_problem(&low).as_deref(), Some("3/100 fresh slots"));
        let stale = PrekeyStatus {
            spk_age_secs: Some(15 * 86_400),
            ..healthy()
        };
        assert!(prekey_problem(&stale).unwrap().contains("15 days"));
        let none = PrekeyStatus {
            spk_age_secs: None,
            ..healthy()
        };
        assert!(prekey_problem(&none).is_some());
    }

    #[test]
    fn one_publish_names_the_head_and_its_slots() {
        let mut written = Written::default();
        for subkey in [3, HEAD, 1, 2] {
            written.add(subkey);
        }
        let Some(CryptoEvent::PrekeyRecordPublished { head, slots }) = written.into_event() else {
            panic!("one event");
        };
        assert!(head);
        assert_eq!(slots, vec![1, 2, 3]);
        assert!(
            Written::default().into_event().is_none(),
            "nothing written, no event"
        );
    }

    #[test]
    fn the_device_list_names_one_device() {
        let list = device_list("VLD0:abc", &"0f".repeat(16)).unwrap();
        let decoded =
            rekindle_codec::capnp_codec::identity::decode_prekey_record_list(&list).unwrap();
        assert_eq!(decoded.devices.len(), 1);
        assert_eq!(decoded.devices[0].record_key, "VLD0:abc");
        assert!(device_list("VLD0:abc", "short").is_err());
    }
}
