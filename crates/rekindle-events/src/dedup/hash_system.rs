//! Content hashers for network transport and system-level events.

use rekindle_types::dto::infoview::InfoviewEvent;
use rekindle_types::dto::skin_library::SkinApplyFailure;
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
            media_route,
            can_write,
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
                *media_route as u8,
                u8::from(*can_write),
            ]);
        }
        NetworkEvent::Strength { bars } => {
            h.update(b"strength|");
            h.update(&[*bars]);
        }
        NetworkEvent::OwnRecordsChanged { phase, error } => {
            h.update(b"own_records|");
            h.update(&[*phase as u8]);
            h.update(error.as_deref().unwrap_or_default().as_bytes());
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

pub(super) fn hash_system(h: &mut blake3::Hasher, s: &SystemEvent, now_ms: u64) {
    let now_bucket = now_ms / 1000 / 10; // 10-second bucketing
    match s {
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
        SystemEvent::AuditChainBroken { cursor } => {
            h.update(b"audit_broken|");
            h.update(&cursor.to_le_bytes());
        }
        SystemEvent::SkinChanged { skin } => {
            h.update(b"skin_changed|");
            h.update(skin.id.as_bytes());
            h.update(b"|");
            h.update(skin.tokens_href.as_bytes());
        }
        SystemEvent::SkinTrialChanged { trial } => {
            h.update(b"skin_trial|");
            if let Some(t) = trial {
                h.update(t.id.as_bytes());
                h.update(b"|");
                h.update(&t.reverts_at_ms.to_le_bytes());
            }
        }
        SystemEvent::SkinApplyFailed {
            window,
            skin,
            cause,
            reverted,
        } => {
            h.update(b"skin_apply_failed|");
            h.update(window.as_bytes());
            h.update(b"|");
            h.update(skin.as_bytes());
            h.update(b"|");
            match cause {
                SkinApplyFailure::ImageDecode { path } => {
                    h.update(b"image|");
                    h.update(path.as_bytes());
                }
                SkinApplyFailure::SheetError { detail } => {
                    h.update(b"sheet|");
                    h.update(detail.as_bytes());
                }
            }
            h.update(&[u8::from(*reverted)]);
        }
        SystemEvent::QuickWipeChanged { armed, shortcut } => {
            // Each committed arm or disarm is its own fact, re-arming the
            // same chord included: the shell registers on every one, so
            // the commit time is part of its identity.
            h.update(b"quick_wipe|");
            h.update(&[u8::from(*armed)]);
            h.update(shortcut.as_deref().unwrap_or("").as_bytes());
            h.update(b"|");
            h.update(&now_ms.to_le_bytes());
        }
        SystemEvent::FavouriteServerAdded {
            game_id,
            server,
            label,
        } => {
            // Each stored add is its own fact (a removed favourite may be
            // added again), so the commit time is part of its identity.
            h.update(b"favourite_added|");
            h.update(&game_id.to_le_bytes());
            h.update(server.as_bytes());
            h.update(b"|");
            h.update(label.as_bytes());
            h.update(b"|");
            h.update(&now_ms.to_le_bytes());
        }
        SystemEvent::ActivityAppended { entries } => {
            // Row ids are unique per identity: the ids are the fact.
            h.update(b"activity|");
            for entry in entries {
                h.update(&entry.id.to_le_bytes());
            }
        }
        SystemEvent::Infoview(InfoviewEvent::Updated { window, view }) => {
            // Each push is a fresh composition: never a replay of the last.
            h.update(b"infoview|");
            h.update(window.as_bytes());
            h.update(b"|");
            h.update(&(view.sections.len() as u64).to_le_bytes());
        }
        SystemEvent::SkinShell { window, op } => {
            // A press is its own fact: the same switch pressed twice runs twice.
            h.update(b"skin_shell|");
            h.update(window.as_bytes());
            h.update(format!("|{op:?}|").as_bytes());
            h.update(&now_ms.to_le_bytes());
        }
        SystemEvent::SkinSignals { window, set } => {
            // A window's signals can return to an earlier set (focus in,
            // out, in again): each change is its own fact.
            h.update(b"skin_signals|");
            h.update(window.as_bytes());
            h.update(format!("|{set:?}|").as_bytes());
            h.update(&now_ms.to_le_bytes());
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
        NotificationEvent::SystemAlert { title, body, .. } => {
            h.update(b"alert|");
            h.update(title.as_bytes());
            h.update(b"|");
            h.update(body.as_bytes());
        }
        NotificationEvent::UpdateAvailable { version } => {
            h.update(b"update|");
            h.update(version.as_bytes());
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
    h.update(c.call_id().unwrap_or_default().as_bytes());
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
        CallEvent::SelfMuteChanged {
            muted,
            timestamp_ms,
        } => {
            h.update(b"selfmute|");
            h.update(&[u8::from(*muted)]);
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
