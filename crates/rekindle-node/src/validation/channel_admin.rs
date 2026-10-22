//! A community's channel and category administration (plan E3.1 E1).

use rekindle_ipc::protocol::IpcRequest;

use super::{channel, community, hex16, name, text, Checked, MAX_TOPIC_LEN};

/// Most channels or categories one reorder names.
const MAX_ORDERED: usize = 1024;
/// Most tags a forum channel carries.
const MAX_FORUM_TAGS: usize = 32;
/// Longest overwrite target: a role id's text or a pseudonym's hex.
const MAX_TARGET_LEN: usize = 128;

fn ordered(ids: &[String], check: impl Fn(&str) -> Checked) -> Checked {
    if ids.len() > MAX_ORDERED {
        return Err(rekindle_ipc::protocol::IpcResponse::error(
            400,
            format!("at most {MAX_ORDERED} ids may be ordered"),
        ));
    }
    ids.iter().try_for_each(|id| check(id))
}

fn target(target_type: &str, target_id: &str) -> Checked {
    if !matches!(target_type, "role" | "member") {
        return Err(rekindle_ipc::protocol::IpcResponse::error(
            400,
            "an overwrite targets a role or a member",
        ));
    }
    text(target_id, MAX_TARGET_LEN, "overwrite target")?;
    if target_id.is_empty() {
        return Err(rekindle_ipc::protocol::IpcResponse::error(
            400,
            "an overwrite names its target",
        ));
    }
    Ok(())
}

/// Channel and category administration: names, ids, tags and targets.
pub(super) fn check(request: &IpcRequest) -> Checked {
    match request {
        IpcRequest::ChannelRename {
            community: c,
            channel_id,
            name: n,
        } => {
            community(c)?;
            channel(channel_id)?;
            name(n, "channel")
        }
        IpcRequest::ChannelMove {
            community: c,
            channel_id,
            category,
        } => {
            community(c)?;
            channel(channel_id)?;
            category
                .as_deref()
                .map_or(Ok(()), |id| hex16(id, "category"))
        }
        IpcRequest::ChannelReorder {
            community: c,
            channel_ids,
        } => {
            community(c)?;
            ordered(channel_ids, channel)
        }
        IpcRequest::ChannelSetTopic {
            community: c,
            channel_id,
            topic,
        } => {
            community(c)?;
            channel(channel_id)?;
            text(topic, MAX_TOPIC_LEN, "topic")
        }
        IpcRequest::ChannelSetForumTags {
            community: c,
            channel_id,
            forum_tags,
        } => {
            community(c)?;
            channel(channel_id)?;
            if forum_tags.len() > MAX_FORUM_TAGS {
                return Err(rekindle_ipc::protocol::IpcResponse::error(
                    400,
                    format!("a forum carries at most {MAX_FORUM_TAGS} tags"),
                ));
            }
            forum_tags.iter().try_for_each(|tag| name(tag, "forum tag"))
        }
        IpcRequest::ChannelOverwriteSet {
            community: c,
            channel_id,
            target_type,
            target_id,
            allow: _,
            deny: _,
        }
        | IpcRequest::ChannelOverwriteDelete {
            community: c,
            channel_id,
            target_type,
            target_id,
        } => {
            community(c)?;
            channel(channel_id)?;
            target(target_type, target_id)
        }
        IpcRequest::CategoryCreate {
            community: c,
            name: n,
        } => {
            community(c)?;
            name(n, "category")
        }
        IpcRequest::CategoryDelete {
            community: c,
            category_id,
        } => {
            community(c)?;
            hex16(category_id, "category")
        }
        IpcRequest::CategoryRename {
            community: c,
            category_id,
            name: n,
        } => {
            community(c)?;
            hex16(category_id, "category")?;
            name(n, "category")
        }
        IpcRequest::CategoryReorder {
            community: c,
            category_ids,
        } => {
            community(c)?;
            ordered(category_ids, |id| hex16(id, "category"))
        }
        other => unreachable!("{} is not a channel request", other.name()),
    }
}
