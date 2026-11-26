//! What lands in our column of a group's rendezvous record (plan decision
//! 67 item 3, design `design-decision67-rv2c-2026-11-25.md` §3): member
//! `i`'s first contact to us, the durable copy of its call, written to
//! subkey `1 + 8i + our slot` by row `i`'s one-time writer. It is opened as
//! any inbound 1:1 frame, with the admission and the roster gate unchanged,
//! and applied only when its sender is the member the creator's signed plan
//! gives row `i` ([`direct::receive_from_member`]).

use rekindle_types::dht_layout::group_rendezvous as layout;

use super::parse_group_id;
use crate::operations::direct::{self, Routed};
use crate::operations::friend::FriendHost;

/// Open `value`, what changed at `subkey` of `group_id`'s rendezvous
/// record. An entry outside our column, or in a row the plan gives no
/// member, is refused and logged; one from another member than the row's
/// is refused by [`direct::receive_from_member`].
pub async fn landed(host: &dyn FriendHost, group_id: &str, subkey: u32, value: &[u8]) {
    let Some(member) = row_member(host, group_id, subkey).await else {
        return;
    };
    if let Routed::Unported(opened) = direct::receive_from_member(host, &member, value).await {
        tracing::debug!(kind = ?opened.type_id, "unported kind in a group rendezvous entry — dropped");
    }
}

/// The member the signed plan gives the row `subkey` is in, when the
/// subkey is in our column of a group we hold a place in.
async fn row_member(host: &dyn FriendHost, group_id: &str, subkey: u32) -> Option<String> {
    let id = parse_group_id(group_id).ok()?;
    let place = match host.group_dm().ok()?.rendezvous(&id).await {
        Ok(place) => place?,
        Err(e) => {
            tracing::warn!(error = %e, "group rendezvous place not read");
            return None;
        }
    };
    let Some((row, column)) = layout::slots_of(subkey).filter(|(_, c)| *c == place.slot) else {
        tracing::warn!(subkey, "refusing a rendezvous entry outside our column");
        return None;
    };
    let member = place.member_at(row).map(str::to_owned);
    if member.is_none() {
        tracing::warn!(
            row,
            column,
            "refusing a rendezvous entry in a row the plan gives no one"
        );
    }
    member
}
