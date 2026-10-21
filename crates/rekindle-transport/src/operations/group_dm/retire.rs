//! Retiring our group chains (plan E2.6, D24, ruling Q2b). A group has no
//! acknowledgements (a `group_dm` row settles `sent` when its entry lands;
//! no `DmAck`, no `ReadUpTo`), so the rule is **age only**: every entry
//! older than `CHAIN_RETENTION` (= `DELIVERY_HORIZON`) is retired, with
//! the ones before it. A member whose cursor is still behind the mark reads
//! a history gap (`receive::GroupSink::history_gap`). It runs after each
//! append and on the friend maintenance's retire tick beside the DM chain's
//! `dm::stream::retire_quiet`.

use rekindle_dm::group::group_id_hex;
use rekindle_records::chain;
use rekindle_records::port::RecordPort;

use super::content::stream_name;
use crate::operations::dm::stream::ParticipantWriter;
use crate::operations::friend::inbox::records;
use crate::operations::friend::{state, FriendHost};

/// Retire our chain in `group_id` (32 hex) through its newest entry older
/// than `CHAIN_RETENTION`. A failure waits for the next append or tick.
pub async fn retire(
    host: &dyn FriendHost,
    group_id: &str,
    ours: &ParticipantWriter,
    port: &dyn RecordPort,
) {
    let horizon = i64::try_from(rekindle_db::repo::chain_entries::CHAIN_RETENTION.as_millis())
        .unwrap_or(i64::MAX);
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let cutoff = host.clock().now_ms_i64().saturating_sub(horizon);
    let (o, stream) = (owner.clone(), stream_name(group_id));
    let through = state::db_call!(host, "read the group retire point", move |conn| {
        rekindle_db::repo::chain_entries::retirable_by_age(conn, &o, &stream, cutoff)
    })
    .await;
    let seq = match through {
        Ok(Some(seq)) => seq,
        Ok(None) => return,
        Err(e) => {
            return tracing::debug!(group = %group_id, error = %e, "group retire point not read")
        }
    };
    if let Err(e) = chain::retire_through(port, &ours.record_key, &ours.writer, seq).await {
        return tracing::debug!(group = %group_id, error = %e, "group chain not retired");
    }
    let stream = stream_name(group_id);
    let forgot = state::db_call!(host, "forget retired group chain entries", move |conn| {
        rekindle_db::repo::chain_entries::forget_through(conn, &owner, &stream, seq)
    })
    .await;
    if let Err(e) = forgot {
        tracing::debug!(group = %group_id, error = %e, "retired group chain entries not forgotten");
    }
    tracing::info!(group = %group_id, through = seq, "group chain retired");
}

/// The retire tick: retire our chain in every group we hold whose chain we
/// hold ([`FriendHost::held_group_chain_writer`]); the tick never creates
/// or announces a record.
pub async fn retire_quiet(host: &dyn FriendHost) {
    let Ok(deps) = host.group_dm() else {
        return;
    };
    let groups = match deps.groups().await {
        Ok(groups) => groups,
        Err(e) => return tracing::debug!(error = %e, "groups not read for retire"),
    };
    let Ok(port) = records(host) else {
        return;
    };
    for group in groups {
        let gid = group_id_hex(&group.group_id);
        match host.held_group_chain_writer(&gid).await {
            Ok(Some(ours)) => retire(host, &gid, &ours, port.as_ref()).await,
            Ok(None) => {}
            Err(e) => tracing::debug!(group = %gid, error = %e, "group chain owner unreadable"),
        }
    }
}
