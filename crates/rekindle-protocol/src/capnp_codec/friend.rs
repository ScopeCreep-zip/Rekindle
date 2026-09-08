use super::{capnp_err, pack, text_or_none, text_to_string, unpack, ProtocolError};
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
    pack(&builder)
}

pub fn decode_request(data: &[u8]) -> Result<FriendRequest, ProtocolError> {
    let reader = unpack(data)?;
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
            if let Some(ref profile) = entry.profile_dht_key {
                fe.set_profile_dht_key(profile.as_str());
            }
            if let Some(ref dm_log) = entry.dm_log_key {
                fe.set_dm_log_key(dm_log.as_str());
            }
        }
    }
    pack(&builder)
}

pub fn decode_friend_list(data: &[u8]) -> Result<Vec<FriendEntry>, ProtocolError> {
    let reader = unpack(data)?;
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

        let nickname = text_or_none(fe.has_nickname(), fe.get_nickname())?;
        let group = text_or_none(fe.has_group_name(), fe.get_group_name())?;

        entries.push(FriendEntry {
            public_key,
            nickname,
            group,
            added_at: fe.get_added_at(),
            profile_dht_key: text_or_none(fe.has_profile_dht_key(), fe.get_profile_dht_key())?,
            dm_log_key: text_or_none(fe.has_dm_log_key(), fe.get_dm_log_key())?,
        });
    }

    Ok(entries)
}
