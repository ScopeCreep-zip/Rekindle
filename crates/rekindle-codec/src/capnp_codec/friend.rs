use super::{capnp_err, pack, text_or_none, unpack, CodecError};
use crate::friend::FriendEntry;
use crate::friend_capnp;

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
        }
    }
    pack(&builder)
}

pub fn decode_friend_list(data: &[u8]) -> Result<Vec<FriendEntry>, CodecError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<friend_capnp::friend_list::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    let list = root.get_friends().map_err(|e| capnp_err(&e))?;
    let mut entries = Vec::with_capacity(list.len() as usize);

    for i in 0..list.len() {
        let fe = list.get(i);
        let public_key_bytes = fe.get_public_key().map_err(|e| capnp_err(&e))?;
        let public_key = String::from_utf8(public_key_bytes.to_vec())
            .map_err(|e| CodecError::Deserialization(format!("invalid UTF-8 public key: {e}")))?;

        let nickname = text_or_none(fe.has_nickname(), fe.get_nickname())?;
        let group = text_or_none(fe.has_group_name(), fe.get_group_name())?;

        entries.push(FriendEntry {
            public_key,
            nickname,
            group,
            added_at: fe.get_added_at(),
            profile_dht_key: text_or_none(fe.has_profile_dht_key(), fe.get_profile_dht_key())?,
        });
    }

    Ok(entries)
}
