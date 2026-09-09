//! Content hashers for network transport and system-level events.

use rekindle_types::subscription_events::{
    CallEvent, NetworkEvent, NotificationEvent, SystemEvent,
};

pub(super) fn hash_network(h: &mut blake3::Hasher, n: &NetworkEvent) {
    match n {
        NetworkEvent::AttachmentChanged {
            attachment_state,
            is_attached,
            public_internet_ready,
            has_route,
        } => {
            h.update(b"attach|");
            // The raw state string is included because it distinguishes
            // transitions the three bits cannot — "attaching" and
            // "attached_weak" both read as not-ready, and collapsing
            // them would dedup away a real change.
            h.update(attachment_state.as_bytes());
            h.update(b"|");
            h.update(&[
                u8::from(*is_attached),
                u8::from(*public_internet_ready),
                u8::from(*has_route),
            ]);
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
        SystemEvent::RaidDetected {
            community,
            joins_in_window,
            ..
        } => {
            // The window count is part of the identity: the detector
            // re-fires as the rate climbs, and each report is a
            // different fact. Bucketed so a burst inside one window
            // does not become a wall of toasts.
            h.update(b"raid_det|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(&joins_in_window.to_le_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        SystemEvent::AutoModAlert {
            community,
            channel,
            message_id,
            rule_name,
        } => {
            // One alert per (message, rule): the same rule matching the
            // same message twice is a duplicate, two different rules
            // matching it are two alerts.
            h.update(b"automod|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(message_id.as_bytes());
            h.update(b"|");
            h.update(rule_name.as_bytes());
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

pub(super) fn hash_notification(h: &mut blake3::Hasher, n: &NotificationEvent) {
    match n {
        NotificationEvent::MessageReceived {
            community_id,
            channel_id,
            title,
            body,
            ..
        } => {
            h.update(b"msg|");
            h.update(community_id.as_bytes());
            h.update(b"|");
            h.update(channel_id.as_bytes());
            h.update(b"|");
            h.update(title.as_bytes());
            h.update(b"|");
            h.update(body.as_bytes());
        }
        NotificationEvent::SystemAlert { title, body } => {
            h.update(b"alert|");
            h.update(title.as_bytes());
            h.update(b"|");
            h.update(body.as_bytes());
        }
        NotificationEvent::UpdateAvailable { version } => {
            h.update(b"update|");
            h.update(version.as_bytes());
        }
        NotificationEvent::SessionResetRequested {
            peer_public_key,
            safety_number,
            ..
        } => {
            h.update(b"reset|");
            h.update(peer_public_key.as_bytes());
            h.update(b"|");
            h.update(safety_number.as_bytes());
        }
        NotificationEvent::CallIncoming { call_id, .. } => {
            // `call_id` alone: a re-delivered ring for the same call is
            // the duplicate this exists to suppress, and `expires_at_ms`
            // would differ between deliveries and defeat it.
            h.update(b"call|");
            h.update(call_id.as_bytes());
        }
    }
}

pub(super) fn hash_call(h: &mut blake3::Hasher, c: &CallEvent) {
    // Call id plus the lifecycle step. A call moves through each step
    // once, so the pair is the identity of the signal — no time bucket
    // is needed and none is wanted: a re-ring for the same call is the
    // duplicate this suppresses.
    h.update(c.call_id().as_bytes());
    h.update(b"|");
    match c {
        CallEvent::Incoming { .. } => h.update(b"incoming"),
        CallEvent::Ringing { .. } => h.update(b"ringing"),
        CallEvent::Started { .. } => h.update(b"started"),
        CallEvent::Connected { .. } => h.update(b"connected"),
        CallEvent::Declined { reason, .. } => {
            h.update(b"declined|");
            h.update(reason.as_bytes())
        }
        CallEvent::Missed { .. } => h.update(b"missed"),
        CallEvent::TimedOut { .. } => h.update(b"timedout"),
        CallEvent::Ended { reason, .. } => {
            h.update(b"ended|");
            h.update(reason.as_bytes())
        }
        // Media toggles and reactions repeat within one call, so they
        // carry their payload into the hash — two different reactions
        // are two events, not a duplicate.
        CallEvent::MediaStateChanged {
            audio,
            video,
            screen,
            timestamp_ms,
            ..
        } => {
            h.update(b"media|");
            h.update(&[u8::from(*audio), u8::from(*video), u8::from(*screen)]);
            h.update(&timestamp_ms.to_le_bytes())
        }
        CallEvent::ReactionReceived {
            sender,
            emoji,
            timestamp_ms,
            ..
        } => {
            h.update(b"reaction|");
            h.update(sender.as_bytes());
            h.update(emoji.as_bytes());
            h.update(&timestamp_ms.to_le_bytes())
        }
        CallEvent::ParticipantJoined {
            participant_pubkey, ..
        } => {
            h.update(b"pjoin|");
            h.update(participant_pubkey.as_bytes())
        }
        CallEvent::ParticipantLeft {
            participant_pubkey,
            reason,
            ..
        } => {
            h.update(b"pleft|");
            h.update(participant_pubkey.as_bytes());
            h.update(reason.as_bytes())
        }
    };
}
