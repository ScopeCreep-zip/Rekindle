//! The 1:1 message envelope and its signing bytes (moved from
//! `rekindle_protocol::messaging`, plan C8).

pub mod envelope;
pub mod signing;

pub use envelope::{
    check_invite_recency, create_invite_blob, decode_invite_url, encode_invite_url,
    verify_invite_blob, AuditLogEntryDto, BannedMemberDto, CategoryDto, ChannelMessageDto,
    EventDto, EventRsvpDto, GameServerDto, InviteBlob, InviteDto, MemberInfoDto, MessageEnvelope,
    MessagePayload, PinnedMessageDto, ReactionGroupDto, RoleDto, Sealing, UnreadCountDto,
};
