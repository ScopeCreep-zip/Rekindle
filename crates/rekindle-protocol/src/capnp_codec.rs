//! Cap'n Proto encode/decode functions bridging Rust domain types to binary wire format.
//!
//! Each sub-module corresponds to a `.capnp` schema file and provides symmetric
//! `encode_*` / `decode_*` pairs. All functions produce packed Cap'n Proto bytes
//! (smaller than unpacked, suitable for DHT storage and Veilid `app_message`).

use crate::error::ProtocolError;

fn capnp_err(e: &capnp::Error) -> ProtocolError {
    ProtocolError::Deserialization(format!("capnp: {e}"))
}

fn not_in_schema(e: capnp::NotInSchema) -> ProtocolError {
    ProtocolError::Deserialization(format!("capnp enum: {e}"))
}

/// Convert a capnp text reader to an owned String.
fn text_to_string(t: capnp::text::Reader<'_>) -> Result<String, ProtocolError> {
    t.to_str()
        .map(std::borrow::ToOwned::to_owned)
        .map_err(|e| ProtocolError::Deserialization(format!("invalid UTF-8 in capnp text: {e}")))
}

// ---------------------------------------------------------------------------
// message.capnp — MessageEnvelope, ChatMessage, Attachment
// ---------------------------------------------------------------------------
pub mod message {
    use super::{capnp_err, text_to_string, ProtocolError};
    use crate::message_capnp;
    use crate::messaging::envelope::{GameInfo, MessageEnvelope};

    /// Encode a `MessageEnvelope` into packed Cap'n Proto bytes.
    pub fn encode_envelope(env: &MessageEnvelope) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<message_capnp::message_envelope::Builder<'_>>();
            root.set_sender_key(&env.sender_key);
            root.set_timestamp(env.timestamp);
            root.set_nonce(&env.nonce);
            root.set_payload(&env.payload);
            root.set_signature(&env.signature);
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    /// Decode packed Cap'n Proto bytes into a `MessageEnvelope`.
    pub fn decode_envelope(data: &[u8]) -> Result<MessageEnvelope, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<message_capnp::message_envelope::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        Ok(MessageEnvelope {
            sender_key: root.get_sender_key().map_err(|e| capnp_err(&e))?.to_vec(),
            timestamp: root.get_timestamp(),
            nonce: root.get_nonce().map_err(|e| capnp_err(&e))?.to_vec(),
            payload: root.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
            signature: root.get_signature().map_err(|e| capnp_err(&e))?.to_vec(),
        })
    }

    /// Encode a chat message body + optional reply-to nonce.
    pub fn encode_chat_message(body: &str, reply_to: Option<&[u8]>) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<message_capnp::chat_message::Builder<'_>>();
            root.set_body(body);
            if let Some(rt) = reply_to {
                root.set_reply_to(rt);
            }
            // attachments left empty for now
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    /// Decode packed bytes into (body, optional `reply_to` nonce).
    pub fn decode_chat_message(data: &[u8]) -> Result<(String, Option<Vec<u8>>), ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<message_capnp::chat_message::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        let body = text_to_string(root.get_body().map_err(|e| capnp_err(&e))?)?;
        let reply_to = if root.has_reply_to() {
            Some(root.get_reply_to().map_err(|e| capnp_err(&e))?.to_vec())
        } else {
            None
        };

        Ok((body, reply_to))
    }

    /// Encode a `GameInfo` into packed Cap'n Proto presence `GameStatus` bytes.
    ///
    /// Re-uses the presence schema's `GameStatus` since it's the same structure.
    pub fn encode_game_info(info: &GameInfo) -> Vec<u8> {
        super::presence::encode_game_status(info)
    }

    /// Decode packed bytes into a `GameInfo`.
    pub fn decode_game_info(data: &[u8]) -> Result<GameInfo, ProtocolError> {
        super::presence::decode_game_status(data)
    }
}

// ---------------------------------------------------------------------------
// presence.capnp — PresenceUpdate, GameStatus
// ---------------------------------------------------------------------------
pub mod presence {
    use super::{capnp_err, text_to_string, ProtocolError};
    use crate::messaging::envelope::GameInfo;
    use crate::presence_capnp;

    /// Encode a presence update (status byte + optional game info).
    pub fn encode_update(status: u8, game: Option<&GameInfo>) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<presence_capnp::presence_update::Builder<'_>>();
            root.set_status(status);
            root.set_timestamp(
                u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis(),
                )
                .unwrap_or(u64::MAX),
            );
            if let Some(g) = game {
                let mut gs = root.init_game_status();
                gs.set_game_id(g.game_id);
                gs.set_game_name(&g.game_name);
                if let Some(ref si) = g.server_info {
                    gs.set_server_info(si.as_str());
                }
                gs.set_elapsed_seconds(g.elapsed_seconds);
            }
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    /// Decode packed bytes into (status, Option<GameInfo>).
    pub fn decode_update(data: &[u8]) -> Result<(u8, Option<GameInfo>), ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<presence_capnp::presence_update::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        let status = root.get_status();
        let game = if root.has_game_status() {
            let gs = root.get_game_status().map_err(|e| capnp_err(&e))?;
            Some(read_game_status(gs)?)
        } else {
            None
        };

        Ok((status, game))
    }

    /// Encode a standalone `GameStatus` (used for DHT subkey 4).
    pub fn encode_game_status(info: &GameInfo) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<presence_capnp::game_status::Builder<'_>>();
            root.set_game_id(info.game_id);
            root.set_game_name(&info.game_name);
            if let Some(ref si) = info.server_info {
                root.set_server_info(si.as_str());
            }
            root.set_elapsed_seconds(info.elapsed_seconds);
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    /// Decode packed bytes into a `GameInfo`.
    pub fn decode_game_status(data: &[u8]) -> Result<GameInfo, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<presence_capnp::game_status::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        read_game_status(root)
    }

    /// Helper: read a `GameStatus` reader into our `GameInfo` struct.
    fn read_game_status(
        gs: presence_capnp::game_status::Reader<'_>,
    ) -> Result<GameInfo, ProtocolError> {
        Ok(GameInfo {
            game_id: gs.get_game_id(),
            game_name: text_to_string(gs.get_game_name().map_err(|e| capnp_err(&e))?)?,
            server_info: if gs.has_server_info() {
                let s = text_to_string(gs.get_server_info().map_err(|e| capnp_err(&e))?)?;
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            },
            elapsed_seconds: gs.get_elapsed_seconds(),
        })
    }
}

// ---------------------------------------------------------------------------
// identity.capnp — UserProfile, PreKeyBundle
// ---------------------------------------------------------------------------
pub mod identity {
    use super::{capnp_err, not_in_schema, text_to_string, ProtocolError};
    use crate::identity_capnp;
    use crate::messaging::envelope::GameInfo;

    /// Domain struct for a user profile (used across account and conversation records).
    #[derive(Debug, Clone)]
    pub struct UserProfile {
        pub display_name: String,
        pub status_message: String,
        pub status: u8, // 0=online, 1=away, 2=busy, 3=offline
        pub avatar_hash: Vec<u8>,
        pub game_status: Option<GameInfo>,
    }

    /// Domain struct for a Signal Protocol pre-key bundle.
    #[derive(Debug, Clone)]
    pub struct PreKeyBundle {
        pub identity_key: Vec<u8>,
        pub signed_pre_key: Vec<u8>,
        pub signed_pre_key_sig: Vec<u8>,
        pub one_time_pre_key: Vec<u8>,
        pub registration_id: u32,
    }

    pub fn encode_profile(profile: &UserProfile) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<identity_capnp::user_profile::Builder<'_>>();
            root.set_display_name(&profile.display_name);
            root.set_status_message(&profile.status_message);

            let status_enum = match profile.status {
                0 => identity_capnp::user_profile::Status::Online,
                1 => identity_capnp::user_profile::Status::Away,
                2 => identity_capnp::user_profile::Status::Busy,
                _ => identity_capnp::user_profile::Status::Offline,
            };
            root.set_status(status_enum);

            if !profile.avatar_hash.is_empty() {
                root.set_avatar_hash(&profile.avatar_hash);
            }

            if let Some(ref g) = profile.game_status {
                let mut gs = root.init_game_status();
                gs.set_game_id(g.game_id);
                gs.set_game_name(&g.game_name);
                if let Some(ref si) = g.server_info {
                    gs.set_server_info(si.as_str());
                }
                gs.set_elapsed_seconds(g.elapsed_seconds);
            }
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    pub fn decode_profile(data: &[u8]) -> Result<UserProfile, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<identity_capnp::user_profile::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        let status = match root.get_status().map_err(not_in_schema)? {
            identity_capnp::user_profile::Status::Online => 0u8,
            identity_capnp::user_profile::Status::Away => 1,
            identity_capnp::user_profile::Status::Busy => 2,
            identity_capnp::user_profile::Status::Offline => 3,
        };

        let game_status = if root.has_game_status() {
            let gs = root.get_game_status().map_err(|e| capnp_err(&e))?;
            Some(GameInfo {
                game_id: gs.get_game_id(),
                game_name: text_to_string(gs.get_game_name().map_err(|e| capnp_err(&e))?)?,
                server_info: if gs.has_server_info() {
                    let s = text_to_string(gs.get_server_info().map_err(|e| capnp_err(&e))?)?;
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                } else {
                    None
                },
                elapsed_seconds: gs.get_elapsed_seconds(),
            })
        } else {
            None
        };

        Ok(UserProfile {
            display_name: text_to_string(root.get_display_name().map_err(|e| capnp_err(&e))?)?,
            status_message: if root.has_status_message() {
                text_to_string(root.get_status_message().map_err(|e| capnp_err(&e))?)?
            } else {
                String::new()
            },
            status,
            avatar_hash: if root.has_avatar_hash() {
                root.get_avatar_hash().map_err(|e| capnp_err(&e))?.to_vec()
            } else {
                Vec::new()
            },
            game_status,
        })
    }

    pub fn encode_prekey_bundle(bundle: &PreKeyBundle) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<identity_capnp::pre_key_bundle::Builder<'_>>();
            root.set_identity_key(&bundle.identity_key);
            root.set_signed_pre_key(&bundle.signed_pre_key);
            root.set_signed_pre_key_sig(&bundle.signed_pre_key_sig);
            root.set_one_time_pre_key(&bundle.one_time_pre_key);
            root.set_registration_id(bundle.registration_id);
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    pub fn decode_prekey_bundle(data: &[u8]) -> Result<PreKeyBundle, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<identity_capnp::pre_key_bundle::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        Ok(PreKeyBundle {
            identity_key: root.get_identity_key().map_err(|e| capnp_err(&e))?.to_vec(),
            signed_pre_key: root
                .get_signed_pre_key()
                .map_err(|e| capnp_err(&e))?
                .to_vec(),
            signed_pre_key_sig: root
                .get_signed_pre_key_sig()
                .map_err(|e| capnp_err(&e))?
                .to_vec(),
            one_time_pre_key: if root.has_one_time_pre_key() {
                root.get_one_time_pre_key()
                    .map_err(|e| capnp_err(&e))?
                    .to_vec()
            } else {
                Vec::new()
            },
            registration_id: root.get_registration_id(),
        })
    }
}

// ---------------------------------------------------------------------------
// friend.capnp — FriendRequest, FriendList, FriendEntry
// ---------------------------------------------------------------------------
pub mod friend {
    use super::{capnp_err, text_to_string, ProtocolError};
    use crate::dht::friends::FriendEntry;
    use crate::friend_capnp;

    /// Domain struct for a friend request payload.
    #[derive(Debug, Clone)]
    pub struct FriendRequest {
        pub sender_key: Vec<u8>,
        pub display_name: String,
        pub message: String,
        pub prekey_bundle: Vec<u8>,
    }

    pub fn encode_request(req: &FriendRequest) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<friend_capnp::friend_request::Builder<'_>>();
            root.set_sender_key(&req.sender_key);
            root.set_display_name(&req.display_name);
            root.set_message(&req.message);
            root.set_pre_key_bundle(&req.prekey_bundle);
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    pub fn decode_request(data: &[u8]) -> Result<FriendRequest, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<friend_capnp::friend_request::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        Ok(FriendRequest {
            sender_key: root.get_sender_key().map_err(|e| capnp_err(&e))?.to_vec(),
            display_name: text_to_string(root.get_display_name().map_err(|e| capnp_err(&e))?)?,
            message: text_to_string(root.get_message().map_err(|e| capnp_err(&e))?)?,
            prekey_bundle: root
                .get_pre_key_bundle()
                .map_err(|e| capnp_err(&e))?
                .to_vec(),
        })
    }

    pub fn encode_friend_list(entries: &[FriendEntry]) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let root = builder.init_root::<friend_capnp::friend_list::Builder<'_>>();
            let mut list = root.init_friends(u32::try_from(entries.len()).unwrap_or(u32::MAX));
            for (i, entry) in entries.iter().enumerate() {
                let mut fe = list.reborrow().get(u32::try_from(i).unwrap_or(u32::MAX));
                fe.set_public_key(entry.public_key.as_bytes());
                if let Some(ref nick) = entry.nickname {
                    fe.set_nickname(nick.as_str());
                }
                if let Some(ref group) = entry.group {
                    fe.set_group_name(group.as_str());
                }
                fe.set_added_at(entry.added_at);
            }
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    pub fn decode_friend_list(data: &[u8]) -> Result<Vec<FriendEntry>, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<friend_capnp::friend_list::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        let list = root.get_friends().map_err(|e| capnp_err(&e))?;
        let mut entries = Vec::with_capacity(list.len() as usize);

        for i in 0..list.len() {
            let fe = list.get(i);
            let public_key_bytes = fe.get_public_key().map_err(|e| capnp_err(&e))?;
            let public_key = String::from_utf8(public_key_bytes.to_vec()).map_err(|e| {
                ProtocolError::Deserialization(format!("invalid UTF-8 public key: {e}"))
            })?;

            let nickname = if fe.has_nickname() {
                let s = text_to_string(fe.get_nickname().map_err(|e| capnp_err(&e))?)?;
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            };

            let group = if fe.has_group_name() {
                let s = text_to_string(fe.get_group_name().map_err(|e| capnp_err(&e))?)?;
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            };

            entries.push(FriendEntry {
                public_key,
                nickname,
                group,
                added_at: fe.get_added_at(),
                profile_dht_key: None, // Not in capnp schema — stored separately
            });
        }

        Ok(entries)
    }
}

// ---------------------------------------------------------------------------
// community.capnp — Community, Channel, Role
// ---------------------------------------------------------------------------
pub mod community {
    use super::{capnp_err, not_in_schema, text_to_string, ProtocolError};
    use crate::community_capnp;
    use crate::dht::community::{
        ChannelEntry, CommunityMetadata, MemberEntry, OverwriteType, PermissionOverwrite,
        RoleDefinition,
    };

    pub fn encode_community(
        meta: &CommunityMetadata,
        channels: &[ChannelEntry],
        roles: &[RoleDefinition],
    ) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<community_capnp::community::Builder<'_>>();
            root.set_name(&meta.name);
            if let Some(ref desc) = meta.description {
                root.set_description(desc.as_str());
            }
            if let Some(ref icon) = meta.icon_hash {
                root.set_icon_hash(icon.as_bytes());
            }
            root.set_created_at(meta.created_at);

            // Channels
            let mut ch_list = root
                .reborrow()
                .init_channels(u32::try_from(channels.len()).unwrap_or(u32::MAX));
            for (i, ch) in channels.iter().enumerate() {
                let mut c = ch_list.reborrow().get(u32::try_from(i).unwrap_or(u32::MAX));
                c.set_id(&ch.id);
                c.set_name(&ch.name);
                let ch_type = match ch.channel_type.as_str() {
                    "voice" => community_capnp::channel::ChannelType::Voice,
                    _ => community_capnp::channel::ChannelType::Text,
                };
                c.set_type(ch_type);
                c.set_sort_order(ch.sort_order);
                if let Some(ref key) = ch.latest_message_key {
                    c.set_latest_message_key(key.as_bytes());
                }
                // Permission overwrites
                if !ch.permission_overwrites.is_empty() {
                    let mut ow_list = c.reborrow().init_permission_overwrites(
                        u32::try_from(ch.permission_overwrites.len()).unwrap_or(u32::MAX),
                    );
                    for (j, ow) in ch.permission_overwrites.iter().enumerate() {
                        let mut o = ow_list.reborrow().get(u32::try_from(j).unwrap_or(u32::MAX));
                        o.set_target_type(match ow.target_type {
                            OverwriteType::Role => {
                                community_capnp::permission_overwrite::OverwriteType::Role
                            }
                            OverwriteType::Member => {
                                community_capnp::permission_overwrite::OverwriteType::Member
                            }
                        });
                        o.set_target_id(&ow.target_id);
                        o.set_allow(ow.allow);
                        o.set_deny(ow.deny);
                    }
                }
            }

            // Roles
            let mut role_list = root
                .reborrow()
                .init_roles(u32::try_from(roles.len()).unwrap_or(u32::MAX));
            for (i, role) in roles.iter().enumerate() {
                let mut r = role_list
                    .reborrow()
                    .get(u32::try_from(i).unwrap_or(u32::MAX));
                r.set_id(role.id);
                r.set_name(&role.name);
                r.set_color(role.color);
                r.set_permissions(role.permissions);
                r.set_position(role.position);
                r.set_hoist(role.hoist);
                r.set_mentionable(role.mentionable);
            }
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    /// Decode a full community record (metadata + channels + roles).
    pub fn decode_community(
        data: &[u8],
        owner_key: &str,
    ) -> Result<(CommunityMetadata, Vec<ChannelEntry>, Vec<RoleDefinition>), ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<community_capnp::community::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        let meta = CommunityMetadata {
            name: text_to_string(root.get_name().map_err(|e| capnp_err(&e))?)?,
            description: if root.has_description() {
                let s = text_to_string(root.get_description().map_err(|e| capnp_err(&e))?)?;
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            },
            icon_hash: if root.has_icon_hash() {
                let bytes = root.get_icon_hash().map_err(|e| capnp_err(&e))?;
                if bytes.is_empty() {
                    None
                } else {
                    Some(String::from_utf8_lossy(bytes).to_string())
                }
            } else {
                None
            },
            created_at: root.get_created_at(),
            owner_key: owner_key.to_string(),
            last_refreshed: 0,
        };

        // Channels
        let ch_reader = root.get_channels().map_err(|e| capnp_err(&e))?;
        let mut channels = Vec::with_capacity(ch_reader.len() as usize);
        for i in 0..ch_reader.len() {
            let c = ch_reader.get(i);
            let channel_type = match c.get_type().map_err(not_in_schema)? {
                community_capnp::channel::ChannelType::Voice => "voice".to_string(),
                community_capnp::channel::ChannelType::Text => "text".to_string(),
            };
            // Decode permission overwrites
            let mut permission_overwrites = Vec::new();
            if c.has_permission_overwrites() {
                let ow_reader = c.get_permission_overwrites().map_err(|e| capnp_err(&e))?;
                for j in 0..ow_reader.len() {
                    let o = ow_reader.get(j);
                    let target_type = match o.get_target_type().map_err(not_in_schema)? {
                        community_capnp::permission_overwrite::OverwriteType::Role => {
                            OverwriteType::Role
                        }
                        community_capnp::permission_overwrite::OverwriteType::Member => {
                            OverwriteType::Member
                        }
                    };
                    permission_overwrites.push(PermissionOverwrite {
                        target_type,
                        target_id: text_to_string(o.get_target_id().map_err(|e| capnp_err(&e))?)?,
                        allow: o.get_allow(),
                        deny: o.get_deny(),
                    });
                }
            }
            channels.push(ChannelEntry {
                id: text_to_string(c.get_id().map_err(|e| capnp_err(&e))?)?,
                name: text_to_string(c.get_name().map_err(|e| capnp_err(&e))?)?,
                channel_type,
                sort_order: c.get_sort_order(),
                latest_message_key: if c.has_latest_message_key() {
                    let bytes = c.get_latest_message_key().map_err(|e| capnp_err(&e))?;
                    if bytes.is_empty() {
                        None
                    } else {
                        Some(String::from_utf8_lossy(bytes).to_string())
                    }
                } else {
                    None
                },
                permission_overwrites,
            });
        }

        // Roles
        let role_reader = root.get_roles().map_err(|e| capnp_err(&e))?;
        let mut roles = Vec::with_capacity(role_reader.len() as usize);
        for i in 0..role_reader.len() {
            let r = role_reader.get(i);
            roles.push(RoleDefinition {
                id: r.get_id(),
                name: text_to_string(r.get_name().map_err(|e| capnp_err(&e))?)?,
                color: r.get_color(),
                permissions: r.get_permissions(),
                position: r.get_position(),
                hoist: r.get_hoist(),
                mentionable: r.get_mentionable(),
            });
        }

        Ok((meta, channels, roles))
    }

    /// Encode just the channel list (for DHT subkey 1).
    pub fn encode_channels(channels: &[ChannelEntry]) -> Vec<u8> {
        // Re-use the community struct with empty name/roles — or encode a bare list.
        // For simplicity, encode the full Community with minimal metadata.
        let meta = CommunityMetadata {
            name: String::new(),
            description: None,
            icon_hash: None,
            created_at: 0,
            owner_key: String::new(),
            last_refreshed: 0,
        };
        encode_community(&meta, channels, &[])
    }

    /// Encode just the role list (for DHT subkey 3).
    pub fn encode_roles(roles: &[RoleDefinition]) -> Vec<u8> {
        let meta = CommunityMetadata {
            name: String::new(),
            description: None,
            icon_hash: None,
            created_at: 0,
            owner_key: String::new(),
            last_refreshed: 0,
        };
        encode_community(&meta, &[], roles)
    }

    /// Encode a member list. Members are not in the community.capnp schema,
    /// so we keep JSON for now (members have `role_ids` which aren't in capnp).
    pub fn encode_members(members: &[MemberEntry]) -> Result<Vec<u8>, ProtocolError> {
        serde_json::to_vec(members).map_err(|e| ProtocolError::Serialization(e.to_string()))
    }

    /// Decode a member list (JSON for now).
    pub fn decode_members(data: &[u8]) -> Result<Vec<MemberEntry>, ProtocolError> {
        serde_json::from_slice(data).map_err(|e| ProtocolError::Deserialization(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// voice.capnp — VoiceSignaling
// ---------------------------------------------------------------------------
pub mod voice {
    use super::{capnp_err, not_in_schema, text_to_string, ProtocolError};
    use crate::voice_capnp;

    /// Voice signaling types matching the capnp enum.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SignalType {
        Join,
        Leave,
        Offer,
        Answer,
        IceCandidate,
    }

    /// Domain struct for voice signaling messages.
    #[derive(Debug, Clone)]
    pub struct VoiceSignaling {
        pub signal_type: SignalType,
        pub channel_id: String,
        pub sender_key: Vec<u8>,
        pub payload: Vec<u8>,
    }

    pub fn encode_signaling(sig: &VoiceSignaling) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<voice_capnp::voice_signaling::Builder<'_>>();
            let capnp_type = match sig.signal_type {
                SignalType::Join => voice_capnp::voice_signaling::SignalType::Join,
                SignalType::Leave => voice_capnp::voice_signaling::SignalType::Leave,
                SignalType::Offer => voice_capnp::voice_signaling::SignalType::Offer,
                SignalType::Answer => voice_capnp::voice_signaling::SignalType::Answer,
                SignalType::IceCandidate => voice_capnp::voice_signaling::SignalType::IceCandidate,
            };
            root.set_type(capnp_type);
            root.set_channel_id(&sig.channel_id);
            root.set_sender_key(&sig.sender_key);
            root.set_payload(&sig.payload);
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    pub fn decode_signaling(data: &[u8]) -> Result<VoiceSignaling, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<voice_capnp::voice_signaling::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        let signal_type = match root.get_type().map_err(not_in_schema)? {
            voice_capnp::voice_signaling::SignalType::Join => SignalType::Join,
            voice_capnp::voice_signaling::SignalType::Leave => SignalType::Leave,
            voice_capnp::voice_signaling::SignalType::Offer => SignalType::Offer,
            voice_capnp::voice_signaling::SignalType::Answer => SignalType::Answer,
            voice_capnp::voice_signaling::SignalType::IceCandidate => SignalType::IceCandidate,
        };

        Ok(VoiceSignaling {
            signal_type,
            channel_id: text_to_string(root.get_channel_id().map_err(|e| capnp_err(&e))?)?,
            sender_key: root.get_sender_key().map_err(|e| capnp_err(&e))?.to_vec(),
            payload: root.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
        })
    }
}

// ---------------------------------------------------------------------------
// account.capnp — AccountHeader, ContactEntry, ChatEntry
// ---------------------------------------------------------------------------
pub mod account {
    use super::{capnp_err, text_to_string, ProtocolError};
    use crate::account_capnp;

    /// Domain struct for the account header stored in the account DHT record.
    #[derive(Debug, Clone)]
    pub struct AccountHeader {
        pub contact_list_key: String,
        pub chat_list_key: String,
        pub invitation_list_key: String,
        pub display_name: String,
        pub status_message: String,
        pub avatar_hash: Vec<u8>,
        pub created_at: u64,
        pub updated_at: u64,
        /// Owner keypair string for the contact list `DHTShortArray` (persisted for re-open).
        pub contact_list_keypair: Option<String>,
        /// Owner keypair string for the chat list `DHTShortArray` (persisted for re-open).
        pub chat_list_keypair: Option<String>,
        /// Owner keypair string for the invitation list `DHTShortArray` (persisted for re-open).
        pub invitation_list_keypair: Option<String>,
    }

    /// Domain struct for a contact entry in the account's contact list.
    #[derive(Debug, Clone)]
    pub struct ContactEntry {
        pub public_key: Vec<u8>,
        pub display_name: String,
        pub nickname: String,
        pub group: String,
        pub local_conversation_key: String,
        pub remote_conversation_key: String,
        pub added_at: u64,
        pub updated_at: u64,
    }

    /// Domain struct for a chat entry in the account's chat list.
    #[derive(Debug, Clone)]
    pub struct ChatEntry {
        pub contact_public_key: Vec<u8>,
        pub local_conversation_key: String,
        pub last_message_timestamp: u64,
        pub unread_count: u32,
        pub is_pinned: bool,
        pub is_muted: bool,
    }

    pub fn encode_account_header(header: &AccountHeader) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<account_capnp::account_header::Builder<'_>>();
            root.set_contact_list_key(&header.contact_list_key);
            root.set_chat_list_key(&header.chat_list_key);
            root.set_invitation_list_key(&header.invitation_list_key);
            root.set_display_name(&header.display_name);
            root.set_status_message(&header.status_message);
            if !header.avatar_hash.is_empty() {
                root.set_avatar_hash(&header.avatar_hash);
            }
            root.set_created_at(header.created_at);
            root.set_updated_at(header.updated_at);
            if let Some(ref kp) = header.contact_list_keypair {
                root.set_contact_list_keypair(kp);
            }
            if let Some(ref kp) = header.chat_list_keypair {
                root.set_chat_list_keypair(kp);
            }
            if let Some(ref kp) = header.invitation_list_keypair {
                root.set_invitation_list_keypair(kp);
            }
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    pub fn decode_account_header(data: &[u8]) -> Result<AccountHeader, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<account_capnp::account_header::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        Ok(AccountHeader {
            contact_list_key: text_to_string(
                root.get_contact_list_key().map_err(|e| capnp_err(&e))?,
            )?,
            chat_list_key: text_to_string(root.get_chat_list_key().map_err(|e| capnp_err(&e))?)?,
            invitation_list_key: text_to_string(
                root.get_invitation_list_key().map_err(|e| capnp_err(&e))?,
            )?,
            display_name: text_to_string(root.get_display_name().map_err(|e| capnp_err(&e))?)?,
            status_message: if root.has_status_message() {
                text_to_string(root.get_status_message().map_err(|e| capnp_err(&e))?)?
            } else {
                String::new()
            },
            avatar_hash: if root.has_avatar_hash() {
                root.get_avatar_hash().map_err(|e| capnp_err(&e))?.to_vec()
            } else {
                Vec::new()
            },
            created_at: root.get_created_at(),
            updated_at: root.get_updated_at(),
            contact_list_keypair: if root.has_contact_list_keypair() {
                let s =
                    text_to_string(root.get_contact_list_keypair().map_err(|e| capnp_err(&e))?)?;
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            },
            chat_list_keypair: if root.has_chat_list_keypair() {
                let s = text_to_string(root.get_chat_list_keypair().map_err(|e| capnp_err(&e))?)?;
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            },
            invitation_list_keypair: if root.has_invitation_list_keypair() {
                let s = text_to_string(
                    root.get_invitation_list_keypair()
                        .map_err(|e| capnp_err(&e))?,
                )?;
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            },
        })
    }

    pub fn encode_contact_entry(entry: &ContactEntry) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<account_capnp::contact_entry::Builder<'_>>();
            root.set_public_key(&entry.public_key);
            root.set_display_name(&entry.display_name);
            root.set_nickname(&entry.nickname);
            root.set_group(&entry.group);
            root.set_local_conversation_key(&entry.local_conversation_key);
            root.set_remote_conversation_key(&entry.remote_conversation_key);
            root.set_added_at(entry.added_at);
            root.set_updated_at(entry.updated_at);
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    pub fn decode_contact_entry(data: &[u8]) -> Result<ContactEntry, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<account_capnp::contact_entry::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        Ok(ContactEntry {
            public_key: root.get_public_key().map_err(|e| capnp_err(&e))?.to_vec(),
            display_name: text_to_string(root.get_display_name().map_err(|e| capnp_err(&e))?)?,
            nickname: if root.has_nickname() {
                text_to_string(root.get_nickname().map_err(|e| capnp_err(&e))?)?
            } else {
                String::new()
            },
            group: if root.has_group() {
                text_to_string(root.get_group().map_err(|e| capnp_err(&e))?)?
            } else {
                String::new()
            },
            local_conversation_key: text_to_string(
                root.get_local_conversation_key()
                    .map_err(|e| capnp_err(&e))?,
            )?,
            remote_conversation_key: if root.has_remote_conversation_key() {
                text_to_string(
                    root.get_remote_conversation_key()
                        .map_err(|e| capnp_err(&e))?,
                )?
            } else {
                String::new()
            },
            added_at: root.get_added_at(),
            updated_at: root.get_updated_at(),
        })
    }

    pub fn encode_chat_entry(entry: &ChatEntry) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<account_capnp::chat_entry::Builder<'_>>();
            root.set_contact_public_key(&entry.contact_public_key);
            root.set_local_conversation_key(&entry.local_conversation_key);
            root.set_last_message_timestamp(entry.last_message_timestamp);
            root.set_unread_count(entry.unread_count);
            root.set_is_pinned(entry.is_pinned);
            root.set_is_muted(entry.is_muted);
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    pub fn decode_chat_entry(data: &[u8]) -> Result<ChatEntry, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<account_capnp::chat_entry::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        Ok(ChatEntry {
            contact_public_key: root
                .get_contact_public_key()
                .map_err(|e| capnp_err(&e))?
                .to_vec(),
            local_conversation_key: text_to_string(
                root.get_local_conversation_key()
                    .map_err(|e| capnp_err(&e))?,
            )?,
            last_message_timestamp: root.get_last_message_timestamp(),
            unread_count: root.get_unread_count(),
            is_pinned: root.get_is_pinned(),
            is_muted: root.get_is_muted(),
        })
    }
}

// ---------------------------------------------------------------------------
// conversation.capnp — ConversationHeader
// ---------------------------------------------------------------------------
pub mod conversation {
    use super::{capnp_err, not_in_schema, text_to_string, ProtocolError};
    use crate::conversation_capnp;
    use crate::identity_capnp;
    use crate::messaging::envelope::GameInfo;

    /// Domain struct for the conversation header stored in a conversation DHT record.
    #[derive(Debug, Clone)]
    pub struct ConversationHeader {
        pub identity_public_key: Vec<u8>,
        pub profile: super::identity::UserProfile,
        pub message_log_key: String,
        pub route_blob: Vec<u8>,
        pub prekey_bundle: super::identity::PreKeyBundle,
        pub created_at: u64,
        pub updated_at: u64,
    }

    pub fn encode_conversation_header(header: &ConversationHeader) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root =
                builder.init_root::<conversation_capnp::conversation_header::Builder<'_>>();
            root.set_identity_public_key(&header.identity_public_key);

            // Write embedded profile
            {
                let mut profile = root.reborrow().init_profile();
                profile.set_display_name(&header.profile.display_name);
                profile.set_status_message(&header.profile.status_message);
                let status_enum = match header.profile.status {
                    0 => identity_capnp::user_profile::Status::Online,
                    1 => identity_capnp::user_profile::Status::Away,
                    2 => identity_capnp::user_profile::Status::Busy,
                    _ => identity_capnp::user_profile::Status::Offline,
                };
                profile.set_status(status_enum);
                if !header.profile.avatar_hash.is_empty() {
                    profile.set_avatar_hash(&header.profile.avatar_hash);
                }
                if let Some(ref g) = header.profile.game_status {
                    let mut gs = profile.init_game_status();
                    gs.set_game_id(g.game_id);
                    gs.set_game_name(&g.game_name);
                    if let Some(ref si) = g.server_info {
                        gs.set_server_info(si.as_str());
                    }
                    gs.set_elapsed_seconds(g.elapsed_seconds);
                }
            }

            root.set_message_log_key(&header.message_log_key);
            root.set_route_blob(&header.route_blob);

            // Write embedded prekey bundle
            {
                let mut pkb = root.reborrow().init_pre_key_bundle();
                pkb.set_identity_key(&header.prekey_bundle.identity_key);
                pkb.set_signed_pre_key(&header.prekey_bundle.signed_pre_key);
                pkb.set_signed_pre_key_sig(&header.prekey_bundle.signed_pre_key_sig);
                if !header.prekey_bundle.one_time_pre_key.is_empty() {
                    pkb.set_one_time_pre_key(&header.prekey_bundle.one_time_pre_key);
                }
                pkb.set_registration_id(header.prekey_bundle.registration_id);
            }

            root.set_created_at(header.created_at);
            root.set_updated_at(header.updated_at);
        }
        let mut output = Vec::new();
        capnp::serialize_packed::write_message(&mut output, &builder)
            .expect("write to Vec never fails");
        output
    }

    fn decode_user_profile(
        profile_reader: identity_capnp::user_profile::Reader<'_>,
    ) -> Result<super::identity::UserProfile, ProtocolError> {
        let status = match profile_reader.get_status().map_err(not_in_schema)? {
            identity_capnp::user_profile::Status::Online => 0u8,
            identity_capnp::user_profile::Status::Away => 1,
            identity_capnp::user_profile::Status::Busy => 2,
            identity_capnp::user_profile::Status::Offline => 3,
        };

        let game_status = if profile_reader.has_game_status() {
            let gs = profile_reader
                .get_game_status()
                .map_err(|e| capnp_err(&e))?;
            Some(GameInfo {
                game_id: gs.get_game_id(),
                game_name: text_to_string(gs.get_game_name().map_err(|e| capnp_err(&e))?)?,
                server_info: if gs.has_server_info() {
                    let s = text_to_string(gs.get_server_info().map_err(|e| capnp_err(&e))?)?;
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                } else {
                    None
                },
                elapsed_seconds: gs.get_elapsed_seconds(),
            })
        } else {
            None
        };

        Ok(super::identity::UserProfile {
            display_name: text_to_string(
                profile_reader
                    .get_display_name()
                    .map_err(|e| capnp_err(&e))?,
            )?,
            status_message: if profile_reader.has_status_message() {
                text_to_string(
                    profile_reader
                        .get_status_message()
                        .map_err(|e| capnp_err(&e))?,
                )?
            } else {
                String::new()
            },
            status,
            avatar_hash: if profile_reader.has_avatar_hash() {
                profile_reader
                    .get_avatar_hash()
                    .map_err(|e| capnp_err(&e))?
                    .to_vec()
            } else {
                Vec::new()
            },
            game_status,
        })
    }

    pub fn decode_conversation_header(data: &[u8]) -> Result<ConversationHeader, ProtocolError> {
        let reader =
            capnp::serialize_packed::read_message(data, capnp::message::ReaderOptions::new())
                .map_err(|e| capnp_err(&e))?;

        let root = reader
            .get_root::<conversation_capnp::conversation_header::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;

        let profile_reader = root.get_profile().map_err(|e| capnp_err(&e))?;
        let profile = decode_user_profile(profile_reader)?;

        // Read embedded prekey bundle
        let pkb_reader = root.get_pre_key_bundle().map_err(|e| capnp_err(&e))?;
        let prekey_bundle = super::identity::PreKeyBundle {
            identity_key: pkb_reader
                .get_identity_key()
                .map_err(|e| capnp_err(&e))?
                .to_vec(),
            signed_pre_key: pkb_reader
                .get_signed_pre_key()
                .map_err(|e| capnp_err(&e))?
                .to_vec(),
            signed_pre_key_sig: pkb_reader
                .get_signed_pre_key_sig()
                .map_err(|e| capnp_err(&e))?
                .to_vec(),
            one_time_pre_key: if pkb_reader.has_one_time_pre_key() {
                pkb_reader
                    .get_one_time_pre_key()
                    .map_err(|e| capnp_err(&e))?
                    .to_vec()
            } else {
                Vec::new()
            },
            registration_id: pkb_reader.get_registration_id(),
        };

        Ok(ConversationHeader {
            identity_public_key: root
                .get_identity_public_key()
                .map_err(|e| capnp_err(&e))?
                .to_vec(),
            profile,
            message_log_key: text_to_string(
                root.get_message_log_key().map_err(|e| capnp_err(&e))?,
            )?,
            route_blob: if root.has_route_blob() {
                root.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec()
            } else {
                Vec::new()
            },
            prekey_bundle,
            created_at: root.get_created_at(),
            updated_at: root.get_updated_at(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::{account, conversation, friend, identity, message, presence, voice};

    #[test]
    fn round_trip_message_envelope() {
        use crate::messaging::envelope::MessageEnvelope;

        let env = MessageEnvelope {
            sender_key: vec![1u8; 32],
            timestamp: 1234567890,
            nonce: vec![42u8; 16],
            payload: b"encrypted content".to_vec(),
            signature: vec![99u8; 64],
        };

        let encoded = message::encode_envelope(&env);
        let decoded = message::decode_envelope(&encoded).unwrap();

        assert_eq!(env.sender_key, decoded.sender_key);
        assert_eq!(env.timestamp, decoded.timestamp);
        assert_eq!(env.nonce, decoded.nonce);
        assert_eq!(env.payload, decoded.payload);
        assert_eq!(env.signature, decoded.signature);
    }

    #[test]
    fn round_trip_chat_message() {
        let (body, reply) = message::decode_chat_message(&message::encode_chat_message(
            "hello world",
            Some(&[1, 2, 3]),
        ))
        .unwrap();
        assert_eq!(body, "hello world");
        assert_eq!(reply, Some(vec![1, 2, 3]));

        let (body2, reply2) =
            message::decode_chat_message(&message::encode_chat_message("no reply", None)).unwrap();
        assert_eq!(body2, "no reply");
        assert_eq!(reply2, None);
    }

    #[test]
    fn round_trip_presence_update() {
        use crate::messaging::envelope::GameInfo;

        let game = GameInfo {
            game_id: 42,
            game_name: "Counter-Strike".to_string(),
            server_info: Some("de_dust2 @ 192.168.1.1:27015".to_string()),
            elapsed_seconds: 3600,
        };

        let encoded = presence::encode_update(1, Some(&game));
        let (status, game_opt) = presence::decode_update(&encoded).unwrap();
        assert_eq!(status, 1);
        let g = game_opt.unwrap();
        assert_eq!(g.game_id, 42);
        assert_eq!(g.game_name, "Counter-Strike");
        assert_eq!(
            g.server_info,
            Some("de_dust2 @ 192.168.1.1:27015".to_string())
        );
        assert_eq!(g.elapsed_seconds, 3600);

        // Without game
        let encoded2 = presence::encode_update(3, None);
        let (status2, game2) = presence::decode_update(&encoded2).unwrap();
        assert_eq!(status2, 3);
        assert!(game2.is_none());
    }

    #[test]
    fn round_trip_user_profile() {
        use crate::messaging::envelope::GameInfo;

        let profile = identity::UserProfile {
            display_name: "xXGamerXx".to_string(),
            status_message: "Playing games".to_string(),
            status: 0,
            avatar_hash: vec![0xDE, 0xAD],
            game_status: Some(GameInfo {
                game_id: 1,
                game_name: "Halo".to_string(),
                server_info: None,
                elapsed_seconds: 120,
            }),
        };

        let encoded = identity::encode_profile(&profile);
        let decoded = identity::decode_profile(&encoded).unwrap();

        assert_eq!(decoded.display_name, "xXGamerXx");
        assert_eq!(decoded.status_message, "Playing games");
        assert_eq!(decoded.status, 0);
        assert_eq!(decoded.avatar_hash, vec![0xDE, 0xAD]);
        let g = decoded.game_status.unwrap();
        assert_eq!(g.game_name, "Halo");
        assert_eq!(g.elapsed_seconds, 120);
    }

    #[test]
    fn round_trip_prekey_bundle() {
        let bundle = identity::PreKeyBundle {
            identity_key: vec![1u8; 32],
            signed_pre_key: vec![2u8; 32],
            signed_pre_key_sig: vec![3u8; 64],
            one_time_pre_key: vec![4u8; 32],
            registration_id: 12345,
        };

        let encoded = identity::encode_prekey_bundle(&bundle);
        let decoded = identity::decode_prekey_bundle(&encoded).unwrap();

        assert_eq!(decoded.identity_key, bundle.identity_key);
        assert_eq!(decoded.signed_pre_key, bundle.signed_pre_key);
        assert_eq!(decoded.signed_pre_key_sig, bundle.signed_pre_key_sig);
        assert_eq!(decoded.one_time_pre_key, bundle.one_time_pre_key);
        assert_eq!(decoded.registration_id, 12345);
    }

    #[test]
    fn round_trip_friend_request() {
        let req = friend::FriendRequest {
            sender_key: vec![0xAA; 32],
            display_name: "FriendlyUser".to_string(),
            message: "Let's be friends!".to_string(),
            prekey_bundle: vec![0xBB; 128],
        };

        let encoded = friend::encode_request(&req);
        let decoded = friend::decode_request(&encoded).unwrap();

        assert_eq!(decoded.sender_key, req.sender_key);
        assert_eq!(decoded.display_name, "FriendlyUser");
        assert_eq!(decoded.message, "Let's be friends!");
        assert_eq!(decoded.prekey_bundle, req.prekey_bundle);
    }

    #[test]
    fn round_trip_friend_list() {
        use crate::dht::friends::FriendEntry;

        let entries = vec![
            FriendEntry {
                public_key: "abc123".to_string(),
                nickname: Some("Buddy".to_string()),
                group: Some("Gaming".to_string()),
                added_at: 1000,
                profile_dht_key: Some("dht_key_1".to_string()),
            },
            FriendEntry {
                public_key: "def456".to_string(),
                nickname: None,
                group: None,
                added_at: 2000,
                profile_dht_key: None,
            },
        ];

        let encoded = friend::encode_friend_list(&entries);
        let decoded = friend::decode_friend_list(&encoded).unwrap();

        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].public_key, "abc123");
        assert_eq!(decoded[0].nickname, Some("Buddy".to_string()));
        assert_eq!(decoded[0].group, Some("Gaming".to_string()));
        assert_eq!(decoded[0].added_at, 1000);
        // profile_dht_key not in capnp schema
        assert_eq!(decoded[0].profile_dht_key, None);

        assert_eq!(decoded[1].public_key, "def456");
        assert_eq!(decoded[1].nickname, None);
        assert_eq!(decoded[1].group, None);
    }

    #[test]
    fn round_trip_voice_signaling() {
        let sig = voice::VoiceSignaling {
            signal_type: voice::SignalType::Offer,
            channel_id: "voice-room-1".to_string(),
            sender_key: vec![0x42; 32],
            payload: b"SDP data here".to_vec(),
        };

        let encoded = voice::encode_signaling(&sig);
        let decoded = voice::decode_signaling(&encoded).unwrap();

        assert_eq!(decoded.signal_type, voice::SignalType::Offer);
        assert_eq!(decoded.channel_id, "voice-room-1");
        assert_eq!(decoded.sender_key, sig.sender_key);
        assert_eq!(decoded.payload, b"SDP data here");
    }

    #[test]
    fn round_trip_game_status() {
        use crate::messaging::envelope::GameInfo;

        let info = GameInfo {
            game_id: 7,
            game_name: "Team Fortress 2".to_string(),
            server_info: Some("2fort @ 10.0.0.1:27015".to_string()),
            elapsed_seconds: 7200,
        };

        let encoded = presence::encode_game_status(&info);
        let decoded = presence::decode_game_status(&encoded).unwrap();

        assert_eq!(decoded.game_id, 7);
        assert_eq!(decoded.game_name, "Team Fortress 2");
        assert_eq!(
            decoded.server_info,
            Some("2fort @ 10.0.0.1:27015".to_string())
        );
        assert_eq!(decoded.elapsed_seconds, 7200);
    }

    #[test]
    fn round_trip_account_header() {
        let header = account::AccountHeader {
            contact_list_key: "VLD0:abc123".to_string(),
            chat_list_key: "VLD0:def456".to_string(),
            invitation_list_key: "VLD0:ghi789".to_string(),
            display_name: "xXGamerXx".to_string(),
            status_message: "Playing games".to_string(),
            avatar_hash: vec![0xDE, 0xAD],
            created_at: 1000,
            updated_at: 2000,
            contact_list_keypair: Some("VLD0:contacts_kp".to_string()),
            chat_list_keypair: Some("VLD0:chats_kp".to_string()),
            invitation_list_keypair: None,
        };

        let encoded = account::encode_account_header(&header);
        let decoded = account::decode_account_header(&encoded).unwrap();

        assert_eq!(decoded.contact_list_key, "VLD0:abc123");
        assert_eq!(decoded.chat_list_key, "VLD0:def456");
        assert_eq!(decoded.invitation_list_key, "VLD0:ghi789");
        assert_eq!(decoded.display_name, "xXGamerXx");
        assert_eq!(decoded.status_message, "Playing games");
        assert_eq!(decoded.avatar_hash, vec![0xDE, 0xAD]);
        assert_eq!(decoded.created_at, 1000);
        assert_eq!(decoded.updated_at, 2000);
        assert_eq!(
            decoded.contact_list_keypair,
            Some("VLD0:contacts_kp".to_string())
        );
        assert_eq!(decoded.chat_list_keypair, Some("VLD0:chats_kp".to_string()));
        assert_eq!(decoded.invitation_list_keypair, None);
    }

    #[test]
    fn round_trip_contact_entry() {
        let entry = account::ContactEntry {
            public_key: vec![0xAA; 32],
            display_name: "Bob".to_string(),
            nickname: "Bobby".to_string(),
            group: "Gaming".to_string(),
            local_conversation_key: "VLD0:local123".to_string(),
            remote_conversation_key: "VLD0:remote456".to_string(),
            added_at: 1000,
            updated_at: 2000,
        };

        let encoded = account::encode_contact_entry(&entry);
        let decoded = account::decode_contact_entry(&encoded).unwrap();

        assert_eq!(decoded.public_key, vec![0xAA; 32]);
        assert_eq!(decoded.display_name, "Bob");
        assert_eq!(decoded.nickname, "Bobby");
        assert_eq!(decoded.group, "Gaming");
        assert_eq!(decoded.local_conversation_key, "VLD0:local123");
        assert_eq!(decoded.remote_conversation_key, "VLD0:remote456");
        assert_eq!(decoded.added_at, 1000);
        assert_eq!(decoded.updated_at, 2000);
    }

    #[test]
    fn round_trip_chat_entry() {
        let entry = account::ChatEntry {
            contact_public_key: vec![0xBB; 32],
            local_conversation_key: "VLD0:chat789".to_string(),
            last_message_timestamp: 3000,
            unread_count: 5,
            is_pinned: true,
            is_muted: false,
        };

        let encoded = account::encode_chat_entry(&entry);
        let decoded = account::decode_chat_entry(&encoded).unwrap();

        assert_eq!(decoded.contact_public_key, vec![0xBB; 32]);
        assert_eq!(decoded.local_conversation_key, "VLD0:chat789");
        assert_eq!(decoded.last_message_timestamp, 3000);
        assert_eq!(decoded.unread_count, 5);
        assert!(decoded.is_pinned);
        assert!(!decoded.is_muted);
    }

    #[test]
    fn round_trip_conversation_header() {
        use crate::messaging::envelope::GameInfo;

        let header = conversation::ConversationHeader {
            identity_public_key: vec![0xCC; 32],
            profile: identity::UserProfile {
                display_name: "Alice".to_string(),
                status_message: "In a meeting".to_string(),
                status: 2, // busy
                avatar_hash: vec![0x01, 0x02],
                game_status: Some(GameInfo {
                    game_id: 42,
                    game_name: "Portal 2".to_string(),
                    server_info: None,
                    elapsed_seconds: 600,
                }),
            },
            message_log_key: "VLD0:msglog123".to_string(),
            route_blob: vec![0xDD; 64],
            prekey_bundle: identity::PreKeyBundle {
                identity_key: vec![1u8; 32],
                signed_pre_key: vec![2u8; 32],
                signed_pre_key_sig: vec![3u8; 64],
                one_time_pre_key: vec![4u8; 32],
                registration_id: 42,
            },
            created_at: 5000,
            updated_at: 6000,
        };

        let encoded = conversation::encode_conversation_header(&header);
        let decoded = conversation::decode_conversation_header(&encoded).unwrap();

        assert_eq!(decoded.identity_public_key, vec![0xCC; 32]);
        assert_eq!(decoded.profile.display_name, "Alice");
        assert_eq!(decoded.profile.status_message, "In a meeting");
        assert_eq!(decoded.profile.status, 2);
        assert_eq!(decoded.profile.avatar_hash, vec![0x01, 0x02]);
        let g = decoded.profile.game_status.unwrap();
        assert_eq!(g.game_name, "Portal 2");
        assert_eq!(g.elapsed_seconds, 600);
        assert_eq!(decoded.message_log_key, "VLD0:msglog123");
        assert_eq!(decoded.route_blob, vec![0xDD; 64]);
        assert_eq!(decoded.prekey_bundle.identity_key, vec![1u8; 32]);
        assert_eq!(decoded.prekey_bundle.registration_id, 42);
        assert_eq!(decoded.created_at, 5000);
        assert_eq!(decoded.updated_at, 6000);
    }
}
