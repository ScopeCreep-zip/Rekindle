use super::{
    bytes_or_empty, capnp_err, pack, text_or_default, text_or_none, text_to_string, unpack,
    ProtocolError,
};
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
    pack(&builder)
}

pub fn decode_account_header(data: &[u8]) -> Result<AccountHeader, ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<account_capnp::account_header::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    Ok(AccountHeader {
        contact_list_key: text_to_string(root.get_contact_list_key().map_err(|e| capnp_err(&e))?)?,
        chat_list_key: text_to_string(root.get_chat_list_key().map_err(|e| capnp_err(&e))?)?,
        invitation_list_key: text_to_string(
            root.get_invitation_list_key().map_err(|e| capnp_err(&e))?,
        )?,
        display_name: text_to_string(root.get_display_name().map_err(|e| capnp_err(&e))?)?,
        status_message: text_or_default(root.has_status_message(), root.get_status_message())?,
        avatar_hash: bytes_or_empty(root.has_avatar_hash(), root.get_avatar_hash())?,
        created_at: root.get_created_at(),
        updated_at: root.get_updated_at(),
        contact_list_keypair: text_or_none(
            root.has_contact_list_keypair(),
            root.get_contact_list_keypair(),
        )?,
        chat_list_keypair: text_or_none(
            root.has_chat_list_keypair(),
            root.get_chat_list_keypair(),
        )?,
        invitation_list_keypair: text_or_none(
            root.has_invitation_list_keypair(),
            root.get_invitation_list_keypair(),
        )?,
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
    pack(&builder)
}

pub fn decode_contact_entry(data: &[u8]) -> Result<ContactEntry, ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<account_capnp::contact_entry::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    Ok(ContactEntry {
        public_key: root.get_public_key().map_err(|e| capnp_err(&e))?.to_vec(),
        display_name: text_to_string(root.get_display_name().map_err(|e| capnp_err(&e))?)?,
        nickname: text_or_default(root.has_nickname(), root.get_nickname())?,
        group: text_or_default(root.has_group(), root.get_group())?,
        local_conversation_key: text_to_string(
            root.get_local_conversation_key()
                .map_err(|e| capnp_err(&e))?,
        )?,
        remote_conversation_key: text_or_default(
            root.has_remote_conversation_key(),
            root.get_remote_conversation_key(),
        )?,
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
    pack(&builder)
}

pub fn decode_chat_entry(data: &[u8]) -> Result<ChatEntry, ProtocolError> {
    let reader = unpack(data)?;
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
