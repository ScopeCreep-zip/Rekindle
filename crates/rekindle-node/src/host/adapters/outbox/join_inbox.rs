//! Durable locator submissions. The row survives until the member record
//! is readable and the inbox has accepted the locator.

use rekindle_governance_runtime::{op_log, op_walk::WalkLogCache};
use rekindle_outbox::{AttemptOutcome, Deadline, Entry, StreamKind};
use rekindle_records::port::{OpenIntent, RecordPort, SetOutcome};
use rekindle_types::{
    dht_layout::join_inbox as layout, governance::OpRef, invite::JoinInboxCapability, key_format,
};
use serde::{Deserialize, Serialize};

use super::OutboxSlot;
use crate::host::adapters::governance::HostGovernance;

pub(super) const SCOPE_PREFIX: &str = "join-inbox/";
pub(super) const STREAM_PREFIX: &str = "dht_write:join-inbox/";

#[derive(Serialize, Deserialize)]
struct Submission {
    community: String,
    capability: JoinInboxCapability,
    locator: OpRef,
    sealed: Vec<u8>,
}

impl OutboxSlot {
    pub(crate) async fn queue_join_inbox(
        &self,
        community: &str,
        capability: JoinInboxCapability,
        locator: OpRef,
        sealed: Vec<u8>,
        expires_at: Option<u64>,
    ) -> Result<(), String> {
        let outbox = self
            .current
            .read()
            .clone()
            .ok_or("join inbox outbox is locked")?;
        let scope = format!("{SCOPE_PREFIX}{}", capability.record_key);
        let client_id = hex::encode(locator.op_hash.0);
        let payload = serde_json::to_vec(&Submission {
            community: community.to_owned(),
            capability,
            locator,
            sealed,
        })
        .map_err(|e| e.to_string())?;
        outbox
            .enqueue(
                Entry {
                    kind: StreamKind::DhtWrite,
                    scope,
                    client_id,
                    payload,
                    deadline: expires_at.map_or(Deadline::Horizon, |secs| {
                        Deadline::At(i64::try_from(secs.saturating_mul(1_000)).unwrap_or(i64::MAX))
                    }),
                },
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub(super) async fn submit(
    host: &HostGovernance,
    pool: &dyn RecordPort,
    stream: &str,
    payload: &[u8],
) -> AttemptOutcome {
    let Ok(row) = serde_json::from_slice::<Submission>(payload) else {
        return AttemptOutcome::Wedged("unreadable join inbox submission".into());
    };
    let capability = &row.capability;
    if stream.strip_prefix(STREAM_PREFIX) != Some(capability.record_key.as_str())
        || !layout::valid_slots(capability.slots)
        || key_format::record_key(&capability.record_key).is_err()
        || row.sealed.is_empty()
        || row.sealed.len() > layout::MAX_ENTRY_LEN
    {
        return AttemptOutcome::Wedged("invalid join inbox submission".into());
    }
    if !host.holds_community(&row.community) {
        return AttemptOutcome::Deferred;
    }
    let Some(latest) = host
        .latest_own_joined(&row.community, &row.locator.author)
        .await
    else {
        return AttemptOutcome::Deferred;
    };
    if latest.hash() != row.locator.op_hash {
        return AttemptOutcome::Superseded("a more recent join has replaced this locator".into());
    }
    if !matches!(
        op_log::fetch_held(host, &row.locator, &WalkLogCache::default()).await,
        Ok(Some(_))
    ) {
        return AttemptOutcome::Deferred;
    }
    let Ok(lease) = pool
        .acquire(
            &capability.record_key,
            Some(capability.writer.as_str()),
            OpenIntent::Check,
        )
        .await
    else {
        return AttemptOutcome::Deferred;
    };
    let outcome = write_empty_slot(pool, lease, &row).await;
    pool.release(lease).await;
    outcome
}

async fn write_empty_slot(
    pool: &dyn RecordPort,
    lease: rekindle_records::lease::LeaseId,
    row: &Submission,
) -> AttemptOutcome {
    let slots = Vec::from_iter(layout::slots(row.capability.slots));
    let Ok(values) = pool.read_hinted(lease, &slots).await else {
        return AttemptOutcome::Deferred;
    };
    if values.iter().any(|(_, value)| value.data == row.sealed) {
        return AttemptOutcome::Sent;
    }
    let hash = blake3::hash(&row.locator.author.0);
    let bytes = hash.as_bytes();
    let start = usize::from(u16::from_le_bytes([bytes[0], bytes[1]])) % slots.len();
    let mut uncertain = false;
    for &slot in slots[start..].iter().chain(&slots[..start]) {
        if values.iter().any(|(subkey, _)| *subkey == slot) {
            continue;
        }
        match pool.get(lease, slot, true).await {
            Ok(Some(value)) if value.data == row.sealed => return AttemptOutcome::Sent,
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(_) => {
                uncertain = true;
                continue;
            }
        }
        match pool.set(lease, slot, row.sealed.clone(), None).await {
            Ok(SetOutcome::Superseded(_)) => continue,
            Ok(SetOutcome::Landed | SetOutcome::Unchanged) => {}
            Ok(SetOutcome::Offline) | Err(_) => return AttemptOutcome::Deferred,
        }
        match pool.get(lease, slot, true).await {
            Ok(Some(value)) if value.data == row.sealed => return AttemptOutcome::Sent,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return AttemptOutcome::Deferred,
        }
    }
    if uncertain {
        AttemptOutcome::Deferred
    } else {
        AttemptOutcome::Superseded("join inbox is full".into())
    }
}
