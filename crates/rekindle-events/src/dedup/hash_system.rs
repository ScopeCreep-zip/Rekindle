//! Content hashers for network transport and system-level events.

use rekindle_types::subscription_events::{NetworkEvent, SystemEvent};

pub(super) fn hash_network(h: &mut blake3::Hasher, n: &NetworkEvent) {
    match n {
        NetworkEvent::AttachmentChanged {
            is_attached,
            public_internet_ready,
        } => {
            h.update(b"attach|");
            h.update(&[u8::from(*is_attached), u8::from(*public_internet_ready)]);
        }
        NetworkEvent::LocalRoutesDied { count } => {
            h.update(b"local_routes|");
            h.update(&(*count as u64).to_le_bytes());
        }
        NetworkEvent::RemoteRoutesDied { peer_keys } => {
            h.update(b"remote_routes|");
            for k in peer_keys {
                h.update(k.as_bytes());
                h.update(b",");
            }
        }
        NetworkEvent::WatchRenewed { record_key } => {
            h.update(b"watch_ok|");
            h.update(record_key.as_bytes());
        }
        NetworkEvent::WatchReestablished { record_key } => {
            h.update(b"watch_re|");
            h.update(record_key.as_bytes());
        }
        NetworkEvent::WatchFailed { record_key, error } => {
            h.update(b"watch_fail|");
            h.update(record_key.as_bytes());
            h.update(b"|");
            h.update(error.as_bytes());
        }
        NetworkEvent::ValueChanged {
            record_key,
            changed_subkeys,
        } => {
            h.update(b"value|");
            h.update(record_key.as_bytes());
            for sk in changed_subkeys {
                h.update(&sk.to_le_bytes());
            }
        }
    }
}

pub(super) fn hash_system(h: &mut blake3::Hasher, s: &SystemEvent) {
    let now_bucket = rekindle_utils::timestamp_secs() / 10; // 10-second bucketing
    match s {
        SystemEvent::Announcement {
            community, body, ..
        } => {
            h.update(b"announce|");
            h.update(community.as_deref().unwrap_or("global").as_bytes());
            h.update(b"|");
            // Hash the body content, not the full body (avoid length-based collisions)
            let body_hash = blake3::hash(body.as_bytes());
            h.update(body_hash.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        SystemEvent::RaidAlert { community, active } => {
            h.update(b"raid|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*active)]);
        }
        SystemEvent::ChannelLockdown { community, locked } => {
            h.update(b"lockdown|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*locked)]);
        }
        SystemEvent::Kicked { community } => {
            h.update(b"kicked|");
            h.update(community.as_bytes());
        }
        SystemEvent::BootstrapRequested {
            community,
            joiner_pseudonym,
        } => {
            h.update(b"boot_req|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(joiner_pseudonym.as_bytes());
        }
        SystemEvent::BootstrapReceived { community } => {
            h.update(b"boot_recv|");
            h.update(community.as_bytes());
        }
        SystemEvent::SyncRequested {
            community,
            channel,
            since_timestamp,
        } => {
            h.update(b"sync_req|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(&since_timestamp.to_le_bytes());
        }
        SystemEvent::SyncReceived {
            community,
            channel,
            message_count,
        } => {
            h.update(b"sync_recv|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(&(*message_count as u64).to_le_bytes());
        }
        SystemEvent::AuditChainBroken { cursor } => {
            h.update(b"audit_broken|");
            h.update(&cursor.to_le_bytes());
        }
    }
}
