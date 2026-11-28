//! Channel and category administration over [`HostGovernance`] (plan E3.1
//! E1): each operation is one or more governance entries written through
//! `rekindle_governance_runtime::apply::write_entry` (signed, gossiped,
//! merged and projected onto the store), then the projected governance is
//! persisted to SQLite (`repo::communities::snapshot`), on both modes.

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{CategoryId, ChannelId};

use super::HostGovernance;

/// One channel or category administration request, ids resolved.
#[derive(Debug, Clone)]
pub(crate) enum ChannelAdminOp {
    Rename {
        channel: ChannelId,
        name: String,
    },
    Move {
        channel: ChannelId,
        category: Option<CategoryId>,
    },
    Reorder {
        channels: Vec<ChannelId>,
    },
    SetTopic {
        channel: ChannelId,
        topic: String,
    },
    SetForumTags {
        channel: ChannelId,
        tags: Vec<String>,
    },
    SetSlowmode {
        channel: ChannelId,
        seconds: u32,
    },
    OverwriteSet {
        channel: ChannelId,
        target_type: String,
        target_id: String,
        allow: u64,
        deny: u64,
    },
    OverwriteDelete {
        channel: ChannelId,
        target_type: String,
        target_id: String,
    },
    CategoryCreate {
        name: String,
    },
    CategoryDelete {
        category: CategoryId,
    },
    CategoryRename {
        category: CategoryId,
        name: String,
    },
    CategoryReorder {
        categories: Vec<CategoryId>,
    },
}

type Entry = GovernanceEntry;

/// A `ChannelUpdated` that changes only what `set` fills in.
fn channel_update(channel: ChannelId, set: impl FnOnce(&mut GovernanceEntry)) -> Entry {
    let mut entry = GovernanceEntry::ChannelUpdated {
        channel_id: channel,
        name: None,
        topic: None,
        forum_tags: None,
        position: None,
        slowmode_seconds: None,
        nsfw: None,
        category_id: None,
    };
    set(&mut entry);
    entry
}

fn position(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

impl HostGovernance {
    /// Apply `op` to `community_id`; the answer is the new category's id
    /// for a create and `null` otherwise.
    ///
    /// # Errors
    /// The community is not loaded, the entry is refused (permission,
    /// bounds), or the write failed.
    pub(crate) async fn channel_admin(
        &self,
        community_id: &str,
        op: ChannelAdminOp,
    ) -> Result<serde_json::Value, GovernanceRuntimeError> {
        let (entries, answer) = self.entries(community_id, op);
        for entry in entries {
            rekindle_governance_runtime::apply::write_entry(self, community_id, entry).await?;
        }
        self.persist_governance_snapshot(community_id).await;
        Ok(answer)
    }

    fn entries(&self, community_id: &str, op: ChannelAdminOp) -> (Vec<Entry>, serde_json::Value) {
        let null = serde_json::Value::Null;
        match op {
            ChannelAdminOp::Rename { channel, name } => (
                vec![channel_update(channel, move |e| {
                    set_channel(e, ChannelChange::Name(name));
                })],
                null,
            ),
            ChannelAdminOp::Move { channel, category } => (
                vec![channel_update(channel, move |e| {
                    set_channel(e, ChannelChange::Category(category));
                })],
                null,
            ),
            ChannelAdminOp::Reorder { channels } => (
                channels
                    .into_iter()
                    .enumerate()
                    .map(|(i, channel)| {
                        channel_update(channel, move |e| {
                            set_channel(e, ChannelChange::Position(position(i)));
                        })
                    })
                    .collect(),
                null,
            ),
            ChannelAdminOp::SetTopic { channel, topic } => (
                vec![channel_update(channel, move |e| {
                    set_channel(e, ChannelChange::Topic(topic));
                })],
                null,
            ),
            ChannelAdminOp::SetForumTags { channel, tags } => (
                vec![channel_update(channel, move |e| {
                    set_channel(e, ChannelChange::ForumTags(tags));
                })],
                null,
            ),
            ChannelAdminOp::SetSlowmode { channel, seconds } => (
                vec![channel_update(channel, move |e| {
                    set_channel(e, ChannelChange::Slowmode(seconds));
                })],
                null,
            ),
            ChannelAdminOp::OverwriteSet {
                channel,
                target_type,
                target_id,
                allow,
                deny,
            } => (
                vec![overwrite(channel, target_type, target_id, allow, deny)],
                null,
            ),
            // A zero overwrite is the merge's "none" (`compute_permissions`
            // applies nothing for it); there is no delete entry.
            ChannelAdminOp::OverwriteDelete {
                channel,
                target_type,
                target_id,
            } => (vec![overwrite(channel, target_type, target_id, 0, 0)], null),
            ChannelAdminOp::CategoryCreate { name } => {
                let mut id = [0u8; 16];
                self.rng().fill(&mut id);
                let next = self.next_category_position(community_id);
                let entry: Entry = GovernanceEntry::CategoryCreated {
                    category_id: CategoryId(id),
                    name,
                    position: next,
                };
                (
                    vec![entry],
                    serde_json::json!({ "categoryId": hex::encode(id) }),
                )
            }
            ChannelAdminOp::CategoryDelete { category } => {
                let entry: Entry = GovernanceEntry::CategoryArchived {
                    category_id: category,
                };
                (vec![entry], null)
            }
            ChannelAdminOp::CategoryRename { category, name } => {
                (vec![category_update(category, Some(name), None)], null)
            }
            ChannelAdminOp::CategoryReorder { categories } => (
                categories
                    .into_iter()
                    .enumerate()
                    .map(|(i, category)| category_update(category, None, Some(position(i))))
                    .collect(),
                null,
            ),
        }
    }

    /// After every live category, as a new channel goes after every channel.
    fn next_category_position(&self, community_id: &str) -> u32 {
        self.governance_state(community_id).map_or(0, |gov| {
            gov.categories
                .values()
                .map(|category| category.position)
                .max()
                .map_or(0, |max| max.saturating_add(1))
        })
    }
}

/// The one field of a `ChannelUpdated` an admin op sets.
enum ChannelChange {
    Name(String),
    Topic(String),
    ForumTags(Vec<String>),
    Position(u32),
    Slowmode(u32),
    /// The category to move into; `None` is uncategorised.
    Category(Option<CategoryId>),
}

fn set_channel(entry: &mut GovernanceEntry, change: ChannelChange) {
    if let GovernanceEntry::ChannelUpdated {
        name,
        topic,
        forum_tags,
        position,
        slowmode_seconds,
        category_id,
        ..
    } = entry
    {
        match change {
            ChannelChange::Name(v) => *name = Some(v),
            ChannelChange::Topic(v) => *topic = Some(v),
            ChannelChange::ForumTags(v) => *forum_tags = Some(v),
            ChannelChange::Position(v) => *position = Some(v),
            ChannelChange::Slowmode(v) => *slowmode_seconds = Some(v),
            ChannelChange::Category(v) => *category_id = Some(v),
        }
    }
}

fn overwrite(
    channel: ChannelId,
    target_type: String,
    target_id: String,
    allow: u64,
    deny: u64,
) -> Entry {
    GovernanceEntry::PermissionOverwrite {
        channel_id: channel,
        target_type,
        target_id,
        allow,
        deny,
    }
}

fn category_update(category: CategoryId, name: Option<String>, position: Option<u32>) -> Entry {
    GovernanceEntry::CategoryUpdated {
        category_id: category,
        name,
        position,
    }
}
