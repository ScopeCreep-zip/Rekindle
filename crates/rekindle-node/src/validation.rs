//! Input validation for the daemon security boundary.
//!
//! [`validate_request`] checks every string and bounded number of an
//! [`IpcRequest`] once, in the host's gate (`host/gate.rs`) for every
//! frontend, before any handler runs: handlers receive only well-formed
//! input. The match is exhaustive, so a
//! new variant cannot reach a handler unvalidated. The CLI may also
//! validate for UX; the daemon enforces.

use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
use rekindle_types::invite::InviteLink;
use rekindle_types::key_format::{
    self, KeyFormatError, MAX_DESCRIPTION_LEN, MAX_NAME_LEN, MAX_NOTE_LEN, MAX_STATUS_LEN,
    MAX_TOPIC_LEN,
};
use rekindle_types::member::MAX_TIMEOUT_SECONDS;

use rekindle_ipc::noise_keys::validate_agent_name;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

/// Largest identity archive (base64) an import takes: the bus's whole
/// application frame (`framing::MAX_APP_FRAME`, 16 MiB).
const MAX_ARCHIVE_LEN: usize = rekindle_ipc::framing::MAX_APP_FRAME as usize;

/// Largest page a history or inbox query returns: Discord's message-list
/// `limit` (1-100).
const MAX_PAGE: u32 = 100;

type Checked = Result<(), IpcResponse>;

/// Map a `key_format` rejection to the 400 the IPC caller sees.
fn reject(label: &str, e: &KeyFormatError) -> IpcResponse {
    IpcResponse::error(400, format!("invalid {label}: {e}"))
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

/// A message body: the shared rule (not blank, bounded, no format
/// characters), as a received one meets too.
fn message_body(body: &str) -> Checked {
    key_format::message_body(body)
        .map(drop)
        .map_err(|e| reject("message body", &e))
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

/// A commit's proof: a typed name within the longest name a subject has.
/// A passphrase is opaque.
fn proof(proof: &rekindle_types::intent::IntentProof) -> Checked {
    match proof {
        rekindle_types::intent::IntentProof::Typed(name) => text(name, MAX_NAME_LEN, "typed name"),
        rekindle_types::intent::IntentProof::Commit
        | rekindle_types::intent::IntentProof::Acknowledged
        | rekindle_types::intent::IntentProof::Passphrase(_) => Ok(()),
    }
}

/// A status a user may pick: Offline is the host's teardown's alone;
/// "Appear Offline" is Invisible, which peers cannot tell from it.
fn presence_status(status: rekindle_types::presence::SessionStatus) -> Checked {
    if status == rekindle_types::presence::SessionStatus::Offline {
        return Err(IpcResponse::error_with_remediation(
            400,
            "offline is set by locking or logging out",
            "pick invisible to appear offline",
        ));
    }
    Ok(())
}

/// An avatar image: not empty, within the bus's frame.
fn avatar(image: &[u8]) -> Checked {
    if image.is_empty() || image.len() > MAX_ARCHIVE_LEN {
        return Err(IpcResponse::error(
            400,
            format!("invalid avatar: 1 to {MAX_ARCHIVE_LEN} bytes"),
        ));
    }
    Ok(())
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

/// Each capability is one of the closed vocabulary
/// (`AgentCapability`), declared once.
fn capabilities(capabilities: &[String]) -> Checked {
    let mut seen = std::collections::HashSet::new();
    for capability in capabilities {
        let Some(parsed) = rekindle_ipc::protocol::AgentCapability::parse(capability) else {
            return Err(IpcResponse::error(
                400,
                format!("unknown capability '{capability}'"),
            ));
        };
        if !seen.insert(parsed) {
            return Err(IpcResponse::error(
                400,
                format!("capability '{capability}' declared twice"),
            ));
        }
    }
    Ok(())
}

/// A new identity's display name, and a passphrase that is not empty.
/// A new identity: a name, when given, is a valid display name (a blank one
/// is refused, not taken for none); the passphrase is not empty.
fn new_identity(display_name: Option<&str>, passphrase: &str) -> Checked {
    if let Some(name) = display_name {
        rekindle_types::presence::limits::display_name(name)
            .map_err(|e| reject("display name", &e))?;
    }
    if passphrase.is_empty() {
        return Err(IpcResponse::error(400, "invalid passphrase: empty"));
    }
    Ok(())
}

/// An identity archive (base64) within the bus's frame.
fn identity_archive(archive: &str) -> Checked {
    if archive.is_empty() || archive.len() > MAX_ARCHIVE_LEN {
        return Err(IpcResponse::error(
            400,
            format!("invalid archive: 1 to {MAX_ARCHIVE_LEN} base64 characters"),
        ));
    }
    Ok(())
}

/// Our own presence, profile and links.
fn own(request: &IpcRequest) -> Checked {
    match request {
        IpcRequest::PresenceSet { status } => presence_status(*status),
        IpcRequest::PresenceSetMessage { message } => {
            if message.is_empty() {
                return Ok(());
            }
            text(message, MAX_STATUS_LEN, "status message")
        }
        IpcRequest::ProfileSetNickname { display_name } => {
            rekindle_types::presence::limits::display_name(display_name)
                .map(drop)
                .map_err(|e| reject("display name", &e))
        }
        // The kind is a closed set; that the bytes are of it is
        // `rekindle-image`'s check, on decode.
        IpcRequest::ProfileSetAvatar { image, kind: _ } => avatar(image),
        IpcRequest::SetReadReceiptsEnabled { .. } => Ok(()),
        IpcRequest::LinkOpen { url } => rekindle_link_preview::https_url(url)
            .map(drop)
            .map_err(|e| IpcResponse::error(400, format!("invalid link: {e}"))),
        other => unreachable!("{} is not one of our own requests", other.name()),
    }
}

/// Validate every field of `request`. The first malformed field answers
/// 400; nothing past this point sees malformed input.
pub fn validate_request(request: &IpcRequest) -> Checked {
    match request {
        // A passphrase is opaque: any bytes may be one (a new identity's
        // may not be empty, below).
        IpcRequest::Lock
        | IpcRequest::Logout
        | IpcRequest::IdentityList
        | IpcRequest::IdentityExport { passphrase: _ }
        | IpcRequest::IdentityRotate { passphrase: _ }
        | IpcRequest::Status
        | IpcRequest::Shutdown
        | IpcRequest::IdentityShow
        | IpcRequest::PrekeyStatus
        | IpcRequest::FriendList
        | IpcRequest::FriendRequests
        | IpcRequest::FriendInviteCreate
        | IpcRequest::FriendCode
        | IpcRequest::FriendTokenRotate
        | IpcRequest::FriendInviteList
        | IpcRequest::FriendBlockedList
        | IpcRequest::FriendGroupDelete { group_id: _ }
        | IpcRequest::CommunityList
        | IpcRequest::PresenceGet
        | IpcRequest::Undo { id: _ }
        | IpcRequest::AuditVerify
        | IpcRequest::AuditExport { since: _ }
        | IpcRequest::GamePresenceClear
        | IpcRequest::VoiceLeave
        | IpcRequest::NetworkStatus
        | IpcRequest::NetworkPeers
        | IpcRequest::PolicyReload
        | IpcRequest::IdentityWipe
        | IpcRequest::IntentCancel { .. }
        // The bus server owns subscriptions; dispatch refuses them.
        | IpcRequest::Subscribe { .. }
        | IpcRequest::Unsubscribe { .. } => Ok(()),

        IpcRequest::Unlock {
            identity,
            passphrase: _,
        }
        | IpcRequest::IdentityDelete { identity } => public_key(identity, "identity"),
        IpcRequest::IdentityCreate {
            display_name,
            passphrase,
        } => new_identity(display_name.as_deref(), passphrase),
        IpcRequest::IdentityImport { archive, .. } => identity_archive(archive),
        IpcRequest::IntentCommit { proof: given, .. } => proof(given),

        IpcRequest::FriendAddFromCode { .. }
        | IpcRequest::FriendAccept { .. }
        | IpcRequest::FriendBlock { .. }
        | IpcRequest::FriendSetNickname { .. }
        | IpcRequest::FriendGroupMove { .. }
        | IpcRequest::FriendReject { .. }
        | IpcRequest::FriendRemove { .. }
        | IpcRequest::FriendCancelRequest { .. }
        | IpcRequest::FriendRetryHandshake { .. }
        | IpcRequest::FriendUnblock { .. }
        | IpcRequest::SessionResetStart { .. }
        | IpcRequest::SessionReinitiate { .. }
        | IpcRequest::SessionResetAccept { .. }
        | IpcRequest::IdentityRotationAccept { .. }
        | IpcRequest::IdentityRotationDecline { .. }
        | IpcRequest::SessionResetDecline { .. }
        | IpcRequest::FriendAddFromInvite { .. }
        | IpcRequest::FriendInviteCancel { .. }
        | IpcRequest::FriendGroupCreate { .. }
        | IpcRequest::FriendGroupRename { .. }
        | IpcRequest::DmSend { .. }
        | IpcRequest::DmTyping { .. }
        | IpcRequest::DmInbox { .. }
        | IpcRequest::DmHistory { .. }
        | IpcRequest::DmMarkRead { .. }
        | IpcRequest::OutboxRetry { .. }
        | IpcRequest::OutboxDiscard { .. } => friends::check(request),
        IpcRequest::AvatarGet { public_key: key }
        | IpcRequest::IdentitySafetyNumber { peer: key }
        | IpcRequest::IdentityMarkVerified { peer: key }
        | IpcRequest::IdentityClearVerification { peer: key } => public_key(key, "public key"),

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
        IpcRequest::ChannelCreate { .. }
        | IpcRequest::ChannelDelete { .. }
        | IpcRequest::ChannelUpdate { .. }
        | IpcRequest::ChannelSend { .. }
        | IpcRequest::ChannelHistory { .. } => channels::check(request),

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

        IpcRequest::PresenceSet { .. }
        | IpcRequest::PresenceSetMessage { .. }
        | IpcRequest::ProfileSetNickname { .. }
        | IpcRequest::SetReadReceiptsEnabled { .. }
        | IpcRequest::ProfileSetAvatar { .. }
        | IpcRequest::LinkOpen { .. } => own(request),
        IpcRequest::GamePresenceSet {
            game_name,
            game_id: _,
            elapsed_seconds: _,
            server_address,
        } => {
            name(game_name, "game")?;
            opt(server_address.as_deref(), |a| {
                rekindle_types::game_server::ServerAddress::parse(a)
                    .map(drop)
                    .map_err(|e| IpcResponse::error(400, format!("invalid server address: {e}")))
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

mod channels;
mod friends;
#[cfg(test)]
mod tests;
