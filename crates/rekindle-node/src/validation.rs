//! Input validation for the daemon security boundary.
//!
//! [`validate_request`] checks every string and bounded number of an
//! [`IpcRequest`] once, in `router::dispatch`, before any handler runs:
//! handlers receive only well-formed input. The match is exhaustive, so a
//! new variant cannot reach a handler unvalidated. The CLI may also
//! validate for UX; the daemon enforces.

use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
use rekindle_types::invite::InviteLink;
use rekindle_types::key_format::{
    self, KeyFormatError, MAX_DESCRIPTION_LEN, MAX_MESSAGE_LEN, MAX_NAME_LEN, MAX_NOTE_LEN,
    MAX_SERVER_ADDRESS_LEN, MAX_STATUS_LEN, MAX_TOPIC_LEN,
};
use rekindle_types::member::MAX_TIMEOUT_SECONDS;

use rekindle_ipc::noise_keys::validate_agent_name;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse, DESTROY_CONFIRMATION, WIPE_CONFIRMATION};

/// Largest page a history or inbox query returns: Discord's message-list
/// `limit` (1-100).
const MAX_PAGE: u32 = 100;

type Checked = Result<(), IpcResponse>;

/// Map a `key_format` rejection to the 400 the IPC caller sees.
fn reject(label: &str, e: &KeyFormatError) -> IpcResponse {
    IpcResponse::error(400, format!("invalid {label}: {e}"))
}

fn record_key(s: &str, label: &str) -> Checked {
    key_format::record_key(s)
        .map(drop)
        .map_err(|e| reject(label, &e))
}

fn public_key(s: &str, label: &str) -> Checked {
    key_format::public_key_hex(s)
        .map(drop)
        .map_err(|e| reject(label, &e))
}

fn pseudonym(s: &str, label: &str) -> Checked {
    key_format::pseudonym_hex(s)
        .map(drop)
        .map_err(|e| reject(label, &e))
}

fn hex16(s: &str, label: &str) -> Checked {
    key_format::hex16_id(s)
        .map(drop)
        .map_err(|e| reject(label, &e))
}

fn community(s: &str) -> Checked {
    key_format::community_ref(s)
        .map(drop)
        .map_err(|e| reject("community", &e))
}

fn channel(s: &str) -> Checked {
    key_format::channel_ref(s)
        .map(drop)
        .map_err(|e| reject("channel", &e))
}

fn name(s: &str, label: &str) -> Checked {
    key_format::name(s, MAX_NAME_LEN)
        .map(drop)
        .map_err(|e| reject(&format!("{label} name"), &e))
}

fn text(s: &str, max: usize, label: &str) -> Checked {
    key_format::text(s, max)
        .map(drop)
        .map_err(|e| reject(label, &e))
}

fn opt<T: ?Sized>(value: Option<&T>, check: impl FnOnce(&T) -> Checked) -> Checked {
    value.map_or(Ok(()), check)
}

/// A key scope on the wire: a channel's 32-hex id, or empty for the
/// community key.
fn key_scope(s: &str) -> Checked {
    if s.is_empty() {
        return Ok(());
    }
    hex16(s, "channel id")
}

fn message_body(body: &str) -> Checked {
    if body.trim().is_empty() {
        return Err(IpcResponse::error(400, "message body cannot be empty"));
    }
    text(body, MAX_MESSAGE_LEN, "message body")
}

fn page(limit: u32) -> Checked {
    if (1..=MAX_PAGE).contains(&limit) {
        Ok(())
    } else {
        Err(IpcResponse::error(
            400,
            format!("limit must be 1-{MAX_PAGE}, got {limit}"),
        ))
    }
}

fn slowmode(seconds: u32) -> Checked {
    if seconds <= MAX_SLOWMODE_SECONDS {
        Ok(())
    } else {
        Err(IpcResponse::error(
            400,
            format!("slowmode must be 0-{MAX_SLOWMODE_SECONDS} seconds, got {seconds}"),
        ))
    }
}

fn confirmation(given: &str, required: &str) -> Checked {
    if given == required {
        Ok(())
    } else {
        Err(IpcResponse::error(
            400,
            format!("confirmation must be exactly '{required}'"),
        ))
    }
}

/// Validate a presence status string: online, away, busy or invisible.
fn status(status: &str) -> Checked {
    match status {
        "online" | "away" | "busy" | "invisible" => Ok(()),
        other => Err(IpcResponse::error(
            400,
            format!("invalid status '{other}' — expected: online, away, busy, invisible"),
        )),
    }
}

/// Validate a channel kind string.
fn channel_kind(kind: &str) -> Checked {
    match kind {
        "text" | "voice" | "announcement" | "forum" | "stage" | "directory" | "media"
        | "events" => Ok(()),
        other => Err(IpcResponse::error(
            400,
            format!(
                "invalid channel kind '{other}' — expected: text, voice, announcement, \
                 forum, stage, directory, media, events"
            ),
        )),
    }
}

fn agent_name(name: &str) -> Checked {
    validate_agent_name(name)
        .map_err(|e| IpcResponse::error(400, format!("invalid agent name: {e}")))
}

/// Each capability is a token in the agent-name grammar, declared once.
/// The set becomes a closed vocabulary when agents gain authority (D1).
fn capabilities(capabilities: &[String]) -> Checked {
    let mut seen = std::collections::HashSet::new();
    for capability in capabilities {
        validate_agent_name(capability)
            .map_err(|e| IpcResponse::error(400, format!("invalid capability: {e}")))?;
        if !seen.insert(capability.as_str()) {
            return Err(IpcResponse::error(
                400,
                format!("capability '{capability}' declared twice"),
            ));
        }
    }
    Ok(())
}

/// Validate every field of `request`. The first malformed field answers
/// 400; nothing past this point sees malformed input.
pub fn validate_request(request: &IpcRequest) -> Checked {
    match request {
        // The passphrase is opaque: any bytes may be one.
        IpcRequest::Unlock { passphrase: _ }
        | IpcRequest::Lock
        | IpcRequest::Status
        | IpcRequest::Shutdown
        | IpcRequest::IdentityShow
        | IpcRequest::IdentityExport
        | IpcRequest::IdentityRotate
        | IpcRequest::FriendList
        | IpcRequest::FriendRequests
        | IpcRequest::CommunityList
        | IpcRequest::PrekeyReplenish
        | IpcRequest::GamePresenceClear
        | IpcRequest::VoiceLeave
        | IpcRequest::NetworkStatus
        | IpcRequest::NetworkPeers
        | IpcRequest::PolicyReload
        // The bus server owns subscriptions; dispatch refuses them.
        | IpcRequest::Subscribe { .. }
        | IpcRequest::Unsubscribe { .. } => Ok(()),

        IpcRequest::IdentityCreate { display_name } => rekindle_types::presence::limits::display_name(display_name)
            .map(drop)
            .map_err(|e| reject("display name", &e)),
        IpcRequest::IdentityDestroy { confirmation: given } => {
            confirmation(given, DESTROY_CONFIRMATION)
        }
        IpcRequest::IdentityWipe { confirmation: given } => confirmation(given, WIPE_CONFIRMATION),

        IpcRequest::FriendAdd { target, message } => {
            record_key(target, "target mailbox key")?;
            text(message, MAX_NOTE_LEN, "message")
        }
        IpcRequest::FriendAccept { public_key: key }
        | IpcRequest::FriendReject { public_key: key }
        | IpcRequest::FriendRemove { public_key: key } => public_key(key, "public key"),

        IpcRequest::CommunityCreate {
            name: community_name,
            description,
            approval_required: _,
        } => {
            name(community_name, "community")?;
            text(description, MAX_DESCRIPTION_LEN, "description")
        }
        IpcRequest::CommunityJoin { invite } => InviteLink::parse(invite).map(drop).map_err(|e| {
            IpcResponse::error(
                400,
                format!(
                    "join requires a full invite link \
                     (rekindle://invite/{{governance_key}}/{{secrets_record_key}}/{{invite_code}}): {e}"
                ),
            )
        }),
        IpcRequest::CommunityLeave { governance_key }
        | IpcRequest::CommunityInfo { governance_key }
        | IpcRequest::CommunityPendingMembers { governance_key } => community(governance_key),
        IpcRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        } => {
            community(governance_key)?;
            pseudonym(member_pseudonym, "member pseudonym")
        }
        IpcRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            reason,
        } => {
            community(governance_key)?;
            pseudonym(member_pseudonym, "member pseudonym")?;
            text(reason, MAX_NOTE_LEN, "reason")
        }
        IpcRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym,
        } => {
            community(governance_key)?;
            pseudonym(new_owner_pseudonym, "new owner pseudonym")
        }

        IpcRequest::ChannelList { community: c }
        | IpcRequest::MekList { community: c }
        | IpcRequest::RoleList { community: c }
        | IpcRequest::BanList { community: c }
        | IpcRequest::InviteList { community: c }
        // Role ids, invite use counts and expiries are governance rules,
        // checked where the op is validated.
        | IpcRequest::RoleDelete {
            community: c,
            role_id: _,
        }
        | IpcRequest::InviteCreate {
            community: c,
            max_uses: _,
            expires_seconds: _,
        } => community(c),
        IpcRequest::ChannelCreate {
            community: c,
            name: channel_name,
            kind,
            category,
            topic,
            slowmode_seconds,
        } => {
            community(c)?;
            name(channel_name, "channel")?;
            channel_kind(kind)?;
            opt(category.as_deref(), |n| name(n, "category"))?;
            opt(topic.as_deref(), |t| text(t, MAX_TOPIC_LEN, "topic"))?;
            slowmode(*slowmode_seconds)
        }
        IpcRequest::ChannelDelete {
            community: c,
            channel_id,
        } => {
            community(c)?;
            channel(channel_id)
        }
        IpcRequest::ChannelUpdate {
            community: c,
            channel_id,
            name: new_name,
            topic,
            slowmode_seconds,
        } => {
            community(c)?;
            channel(channel_id)?;
            opt(new_name.as_deref(), |n| name(n, "channel"))?;
            opt(topic.as_deref(), |t| text(t, MAX_TOPIC_LEN, "topic"))?;
            opt(slowmode_seconds.as_ref(), |s| slowmode(*s))
        }
        IpcRequest::ChannelSend {
            community: c,
            channel: ch,
            body,
            reply_to: _,
        } => {
            community(c)?;
            channel(ch)?;
            message_body(body)
        }
        IpcRequest::ChannelHistory {
            community: c,
            channel: ch,
            limit,
        } => {
            community(c)?;
            channel(ch)?;
            page(*limit)
        }

        IpcRequest::DmSend { peer_key, body } => {
            public_key(peer_key, "peer key")?;
            message_body(body)
        }
        IpcRequest::DmTyping {
            peer_key,
            typing: _,
        } => public_key(peer_key, "peer key"),
        IpcRequest::DmInbox { limit } => page(*limit),

        IpcRequest::MekRotate {
            community: c,
            channel: ch,
        }
        | IpcRequest::MekRequest {
            community: c,
            channel: ch,
            generation: _,
        } => {
            community(c)?;
            key_scope(ch)
        }

        IpcRequest::PresenceSet { status: s, message } => {
            status(s)?;
            opt(message.as_deref(), |m| text(m, MAX_STATUS_LEN, "status message"))
        }
        IpcRequest::GamePresenceSet {
            game_name,
            game_id: _,
            elapsed_seconds: _,
            server_address,
        } => {
            name(game_name, "game")?;
            opt(server_address.as_deref(), |a| {
                key_format::name(a, MAX_SERVER_ADDRESS_LEN)
                    .map(drop)
                    .map_err(|e| reject("server address", &e))
            })
        }

        IpcRequest::RoleCreate {
            community: c,
            name: role_name,
            permissions: _,
            color: _,
            position: _,
        } => {
            community(c)?;
            name(role_name, "role")
        }
        IpcRequest::RoleUpdate {
            community: c,
            role_id: _,
            name: new_name,
            permissions: _,
            color: _,
        } => {
            community(c)?;
            opt(new_name.as_deref(), |n| name(n, "role"))
        }
        IpcRequest::RoleAssign {
            community: c,
            member_pseudonym,
            role_id: _,
        }
        | IpcRequest::RoleUnassign {
            community: c,
            member_pseudonym,
            role_id: _,
        } => {
            community(c)?;
            pseudonym(member_pseudonym, "member pseudonym")
        }

        IpcRequest::Kick {
            community: c,
            target_pseudonym,
        }
        | IpcRequest::Unban {
            community: c,
            target_pseudonym,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")
        }
        IpcRequest::Ban {
            community: c,
            target_pseudonym,
            reason,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")?;
            opt(reason.as_deref(), |r| text(r, MAX_NOTE_LEN, "reason"))
        }
        IpcRequest::Timeout {
            community: c,
            target_pseudonym,
            duration_seconds,
            reason,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")?;
            if !(1..=MAX_TIMEOUT_SECONDS).contains(duration_seconds) {
                return Err(IpcResponse::error(
                    400,
                    format!(
                        "timeout must be 1-{MAX_TIMEOUT_SECONDS} seconds, got {duration_seconds}"
                    ),
                ));
            }
            opt(reason.as_deref(), |r| text(r, MAX_NOTE_LEN, "reason"))
        }

        IpcRequest::InviteRevoke {
            community: c,
            invite_code,
        } => {
            community(c)?;
            key_format::hex16_id(invite_code).map(drop).map_err(|e| {
                IpcResponse::error(
                    400,
                    format!(
                        "invalid invite id: {e} — revoke takes the 16-byte invite id from \
                         `invite list`, not the raw code, which is never published"
                    ),
                )
            })
        }

        IpcRequest::VoiceJoin {
            community: c,
            channel: ch,
            muted: _,
            deafened: _,
        } => {
            community(c)?;
            hex16(ch, "voice channel id")
        }

        IpcRequest::AgentRegister {
            name: agent,
            agent_type: _,
            capabilities: caps,
        } => {
            agent_name(agent)?;
            capabilities(caps)
        }
        IpcRequest::AgentRevoke { name: agent } => agent_name(agent),
    }
}

#[cfg(test)]
mod tests;
