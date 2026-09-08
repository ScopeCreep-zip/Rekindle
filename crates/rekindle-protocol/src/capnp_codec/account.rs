use super::{
    bytes_or_empty, capnp_err, pack, text_or_default, text_to_string, unpack, ProtocolError,
};
use crate::account_capnp;

/// Domain struct for the account header stored in the account DHT record.
///
/// Held pointers and owner keypairs for three child `DHTShortArray`s
/// (contacts, chats, invitations) until it was established that nothing
/// wrote to or read from any of them — see `schemas/account.capnp`.
#[derive(Debug, Clone)]
pub struct AccountHeader {
    pub display_name: String,
    pub status_message: String,
    pub avatar_hash: Vec<u8>,
    pub created_at: u64,
    pub updated_at: u64,
}

pub fn encode_account_header(header: &AccountHeader) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<account_capnp::account_header::Builder<'_>>();
        root.set_display_name(&header.display_name);
        root.set_status_message(&header.status_message);
        if !header.avatar_hash.is_empty() {
            root.set_avatar_hash(&header.avatar_hash);
        }
        root.set_created_at(header.created_at);
        root.set_updated_at(header.updated_at);
    }
    pack(&builder)
}

pub fn decode_account_header(data: &[u8]) -> Result<AccountHeader, ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<account_capnp::account_header::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    Ok(AccountHeader {
        display_name: text_to_string(root.get_display_name().map_err(|e| capnp_err(&e))?)?,
        status_message: text_or_default(root.has_status_message(), root.get_status_message())?,
        avatar_hash: bytes_or_empty(root.has_avatar_hash(), root.get_avatar_hash())?,
        created_at: root.get_created_at(),
        updated_at: root.get_updated_at(),
    })
}
