//! Friend invite blobs and the community RPC DTOs. The 1:1 wire is
//! `capnp_codec::direct` (plan E2.3).

pub mod envelope;

pub use envelope::{
    check_invite_recency, create_invite_blob, decode_invite_url, encode_invite_url,
    verify_invite_blob, AuditLogEntryDto, BannedMemberDto, CategoryDto, ChannelMessageDto,
    EventDto, EventRsvpDto, GameServerDto, InviteBlob, InviteDto, MemberInfoDto, PinnedMessageDto,
    ReactionGroupDto, RoleDto, UnreadCountDto,
};
