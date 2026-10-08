//! Unit tests for the CRDT merge engine, grouped by entry category.

use super::*;
use rekindle_types::id::{ChannelId, RoleId, ThreadId};

pub(super) fn pseudo(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

pub(super) fn role_id(b: u8) -> RoleId {
    RoleId([b; 16])
}

pub(super) fn channel_id(b: u8) -> ChannelId {
    ChannelId([b; 16])
}

mod admission;
mod channels;
mod community;
mod events;
mod expression;
mod general;
mod moderation;
mod roles;
