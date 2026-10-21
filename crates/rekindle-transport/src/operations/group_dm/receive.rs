//! Reading the members' group chains (plan E2.6 G2c), on the `rekindle-sync`
//! stream engine like the DM chain: the stream is `group_dm:<group_id>`,
//! the writer the member, the record the chain the member announced
//! (`group_dm_chains`). Each entry opens through `rekindle_dm::open_received`
//! under the member's key of its epoch: an older epoch than held is
//! refused and skipped, a newer one waits (the cursor stays) for the key's
//! `GroupDmAccept`, which diffs again. An opened message is a `group_dm`
//! `messages` row under the group's id with the member as its sender.
//!
//! The triggers mirror the DM chain's: a member's hint ([`on_hint`]), its
//! chain's watch ([`on_watch_change`], opened by [`watch_all`] at unlock and
//! [`watch_member`] when a chain is announced) and [`catch_up_all`] at
//! unlock. A member who left has no chain or key held (their removal
//! deleted both), and after our own leave we hold no group: neither is
//! read again.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, GroupContent, GroupId};
use rekindle_dm::group::group_id_hex;
use rekindle_dm::DmError;
use rekindle_records::chain::ChainEntry;
use rekindle_sync::stream::{SinkError, StreamSink};
use rekindle_types::message::{ConversationType, SendState};
use rekindle_types::subscription_events::{
    GroupDmEvent, GroupDmMember, SubscriptionEvent, TypingContext,
};

use super::content::{parse_inner, stream_name};
use crate::operations::dm::client_id_text;
use crate::operations::friend::inbox::records;
use crate::operations::friend::{state, FriendHost};
use crate::operations::typing;

/// How long a member's typing start shows without a refresh.
const TYPING_TTL_MS: u64 = rekindle_events::typing::TYPING_TTL_MS;

/// The key a member's group chain is watched under (the watch table is
/// by conversation; a group's is per member).
#[must_use]
pub fn watch_key(group_id: &str, member: &str) -> String {
    format!("{}:{member}", stream_name(group_id))
}

/// A [`watch_key`] read back: `(group id hex, member)`.
#[must_use]
pub fn parse_watch_key(key: &str) -> Option<(String, String)> {
    let rest = key.strip_prefix("group_dm:")?;
    let (group, member) = rest.split_once(':')?;
    Some((group.to_owned(), member.to_owned()))
}

fn typing_context(group_id: &GroupId) -> TypingContext {
    TypingContext::GroupDm {
        group_id: group_id_hex(group_id),
    }
}

/// A member's hint over their pairwise session: typing is shown (never
/// stored); any other entry makes us diff their chain, which is where
/// content is read from. Whether it applied.
pub async fn on_hint(host: &dyn FriendHost, sender: &str, content: &GroupContent) -> bool {
    let Ok(deps) = host.group_dm() else {
        return false;
    };
    match rekindle_dm::open_received(deps.as_ref(), sender, content).await {
        Ok(plain) => {
            if let Ok((DirectBody::Typing(t), _)) = parse_inner(&plain) {
                let ctx = typing_context(&content.group_id);
                if t.typing {
                    typing::started(host, ctx, sender, TYPING_TTL_MS);
                } else {
                    typing::stopped(host, &ctx, sender);
                }
                return true;
            }
            spawn_diff(host, &content.group_id, sender);
            true
        }
        // Not yet our key of its epoch: the chain is read once it lands.
        Err(DmError::KeyMissing { .. }) => {
            spawn_diff(host, &content.group_id, sender);
            true
        }
        Err(e) => {
            tracing::info!(from = %sender, error = %e, "group hint refused");
            false
        }
    }
}

/// Diff `member`'s chain in `group_id` in the background.
pub fn spawn_diff(host: &dyn FriendHost, group_id: &GroupId, member: &str) {
    let (owned, id, member) = (host.owned(), *group_id, member.to_string());
    host.scope().spawn_or_drop("group stream diff", async move {
        diff(owned.as_ref(), &id, &member).await;
    });
}

/// Watch `member`'s announced chain `record` in `group_id` (unlock, and
/// each announcement).
pub async fn watch_member(host: &dyn FriendHost, group_id: &GroupId, member: &str, record: &str) {
    host.watch_participant(&watch_key(&group_id_hex(group_id), member), record)
        .await;
}

/// Every member chain of every group we hold, `(group, member, record)`.
async fn member_chains(host: &dyn FriendHost) -> Vec<(GroupId, String, String)> {
    let Ok(deps) = host.group_dm() else {
        return Vec::new();
    };
    let me = deps.own_identity();
    let groups = match deps.groups().await {
        Ok(groups) => groups,
        Err(e) => {
            tracing::debug!(error = %e, "groups not read");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for g in groups {
        match deps.chains(&g.group_id).await {
            Ok(chains) => out.extend(
                chains
                    .into_iter()
                    .filter(|(m, _)| *m != me)
                    .map(|(m, r)| (g.group_id, m, r)),
            ),
            Err(e) => tracing::debug!(error = %e, "group chains not read"),
        }
    }
    out
}

/// Watch every member chain of every group we hold (unlock, heal).
pub async fn watch_all(host: &dyn FriendHost) {
    for (group, member, record) in member_chains(host).await {
        watch_member(host, &group, &member, &record).await;
    }
}

/// Diff every member chain of every group we hold (unlock, heal).
pub async fn catch_up_all(host: &dyn FriendHost) {
    for (group, member, _) in member_chains(host).await {
        diff(host, &group, &member).await;
    }
}

/// The watch on `key` ([`watch_key`]) saw `record` change: diff it when it
/// is the chain we hold for that member.
pub async fn on_watch_change(host: &dyn FriendHost, key: &str, record: &str) {
    let Some((group, member)) = parse_watch_key(key) else {
        return;
    };
    let Ok(id) = super::parse_group_id(&group) else {
        return;
    };
    let Ok(deps) = host.group_dm() else {
        return;
    };
    match deps.chain(&id, &member).await {
        Ok(Some(held)) if held == record => diff(host, &id, &member).await,
        _ => tracing::debug!(member = %member, "change on a group chain we do not read"),
    }
}

/// Diff `member`'s chain in `group_id` from our cursor; a failure waits for
/// the next trigger, which resumes from the committed cursor.
pub async fn diff(host: &dyn FriendHost, group_id: &GroupId, member: &str) {
    let Ok(deps) = host.group_dm() else {
        return;
    };
    let record = match deps.chain(group_id, member).await {
        Ok(Some(record)) => record,
        Ok(None) => return tracing::trace!(member = %member, "no group chain of theirs"),
        Err(e) => return tracing::debug!(member = %member, error = %e, "group diff skipped"),
    };
    let Ok(port) = records(host) else {
        return tracing::debug!(member = %member, "group diff waits for the records pool");
    };
    let sink = GroupSink {
        host,
        group_id: *group_id,
    };
    let stream = stream_name(&group_id_hex(group_id));
    match host
        .stores()
        .streams
        .diff(port.as_ref(), &sink, &stream, member, &record)
        .await
    {
        Ok(d) => tracing::debug!(member = %member, applied = d.applied, "group stream diffed"),
        Err(e) => tracing::debug!(member = %member, error = %e, "group stream diff failed"),
    }
}

/// A group stream's sink: `open_received`, `messages` and `stream_cursors`.
struct GroupSink<'a> {
    host: &'a dyn FriendHost,
    group_id: GroupId,
}

fn sink_error(e: impl std::fmt::Display) -> SinkError {
    SinkError(e.to_string())
}

impl GroupSink<'_> {
    async fn advance(&self, stream: &str, writer: &str, seq: u64) -> Result<(), SinkError> {
        let owner = state::owner(self.host).map_err(sink_error)?;
        let (stream, writer, now) = (
            stream.to_string(),
            writer.to_string(),
            self.host.clock().now_ms_i64(),
        );
        state::db_call!(self.host, "commit group cursor", move |conn| {
            rekindle_db::repo::stream_cursors::advance(conn, &owner, &stream, &writer, seq, now)
        })
        .await
        .map(|_| ())
        .map_err(sink_error)
    }

    /// Store `writer`'s opened entry `seq`; a replay finds it held.
    async fn store(&self, writer: &str, plain: &[u8], seq: u64) -> Result<(), SinkError> {
        let (body, sent_at_ms) = match parse_inner(plain) {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::info!(from = %writer, error = %e, "group entry unreadable; skipped");
                return Ok(());
            }
        };
        let DirectBody::DmText(text) = body else {
            tracing::debug!(from = %writer, kind = ?body.type_id(), "group kind not stored");
            return Ok(());
        };
        let owner = state::owner(self.host).map_err(sink_error)?;
        let gid = group_id_hex(&self.group_id);
        let message_id = client_id_text(&text.client_id);
        let reply_to = text.reply_to.as_ref().map(client_id_text);
        let timestamp = i64::try_from(sent_at_ms).unwrap_or(i64::MAX);
        let (group, author, text_body, mid, parent) = (
            gid.clone(),
            writer.to_string(),
            text.body.clone(),
            message_id.clone(),
            reply_to.clone(),
        );
        let stream_seq = i64::try_from(seq).ok();
        let stored = state::db_call!(self.host, "store group message", move |conn| {
            rekindle_db::repo::messages::insert_dm(
                conn,
                &rekindle_db::repo::messages::NewDm {
                    owner_key: &owner,
                    peer_key: &group,
                    conversation_type: ConversationType::GroupDm,
                    sender_key: &author,
                    body: &text_body,
                    timestamp,
                    is_read: false,
                    message_id: &mid,
                    reply_to: parent.as_deref(),
                    send_state: SendState::Delivered,
                    stream_seq,
                },
            )
        })
        .await
        .map_err(sink_error)?;
        if !stored.inserted {
            return Ok(());
        }
        typing::stopped(self.host, &typing_context(&self.group_id), writer);
        let deps = self.host.group_dm().map_err(sink_error)?;
        self.host
            .publish(SubscriptionEvent::GroupDm(GroupDmEvent::Message {
                group_id: gid,
                message_id,
                sender: GroupDmMember {
                    public_key: writer.to_string(),
                    display_name: deps.display_name(writer).await,
                },
                body: text.body,
                timestamp,
                reply_to,
            }));
        Ok(())
    }
}

#[async_trait::async_trait]
impl StreamSink for GroupSink<'_> {
    async fn cursor(&self, stream: &str, writer: &str) -> Result<u64, SinkError> {
        let owner = state::owner(self.host).map_err(sink_error)?;
        let (stream, writer) = (stream.to_string(), writer.to_string());
        state::db_call!(self.host, "read group cursor", move |conn| {
            rekindle_db::repo::stream_cursors::get(conn, &owner, &stream, &writer)
        })
        .await
        .map_err(sink_error)
    }

    /// An entry that can never open (not a `GroupContent` of this group,
    /// an older epoch, a forged seal) is skipped; one at a newer epoch
    /// stops the diff until its key lands.
    async fn apply(&self, stream: &str, writer: &str, entry: &ChainEntry) -> Result<(), SinkError> {
        let content = match DirectBody::decode(DmTypeId::GroupContent, &entry.ciphertext) {
            Ok(DirectBody::GroupContent(c)) if c.group_id == self.group_id => c,
            _ => {
                tracing::info!(from = %writer, seq = entry.seq, "not this group's entry; skipped");
                return self.advance(stream, writer, entry.seq).await;
            }
        };
        let deps = self.host.group_dm().map_err(sink_error)?;
        match rekindle_dm::open_received(deps.as_ref(), writer, &content).await {
            Ok(plain) => self.store(writer, &plain, entry.seq).await?,
            Err(e @ (DmError::KeyMissing { .. } | DmError::Storage(_))) => {
                return Err(sink_error(e));
            }
            Err(e) => {
                tracing::info!(from = %writer, seq = entry.seq, error = %e, "group entry refused");
            }
        }
        self.advance(stream, writer, entry.seq).await
    }

    /// The member retired entries before we read them: the cursor moves
    /// past them (a group shows no gap marker; its members' history is
    /// each member's own chain).
    async fn history_gap(&self, stream: &str, writer: &str, through: u64) -> Result<(), SinkError> {
        tracing::info!(from = %writer, through, "group chain retired past our cursor");
        self.advance(stream, writer, through).await
    }

    async fn caught_up(&self, _stream: &str, _writer: &str) {}
}
