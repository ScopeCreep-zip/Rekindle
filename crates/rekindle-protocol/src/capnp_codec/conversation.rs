
use super::{
    bytes_or_empty, capnp_err, pack, read_profile, text_to_string, unpack, write_profile,
    ProtocolError,
};
use crate::conversation_capnp;

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
        let mut root = builder.init_root::<conversation_capnp::conversation_header::Builder<'_>>();
        root.set_identity_public_key(&header.identity_public_key);

        // Write embedded profile
        write_profile(root.reborrow().init_profile(), &header.profile);

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
    pack(&builder)
}

pub fn decode_conversation_header(data: &[u8]) -> Result<ConversationHeader, ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<conversation_capnp::conversation_header::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;

    let profile = read_profile(root.get_profile().map_err(|e| capnp_err(&e))?)?;

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
        one_time_pre_key: bytes_or_empty(
            pkb_reader.has_one_time_pre_key(),
            pkb_reader.get_one_time_pre_key(),
        )?,
        one_time_pre_key_id: pkb_reader.get_one_time_pre_key_id(),
        registration_id: pkb_reader.get_registration_id(),
        pqpk_lr: pkb_reader
            .get_pqpk_lr()
            .map_err(|e| capnp_err(&e))?
            .to_vec(),
        pqpk_lr_sig: pkb_reader
            .get_pqpk_lr_sig()
            .map_err(|e| capnp_err(&e))?
            .to_vec(),
        pqpk_ot: bytes_or_empty(pkb_reader.has_pqpk_ot(), pkb_reader.get_pqpk_ot())?,
        pqpk_ot_sig: bytes_or_empty(pkb_reader.has_pqpk_ot_sig(), pkb_reader.get_pqpk_ot_sig())?,
        pqpk_ot_id: pkb_reader.get_pqpk_ot_id(),
    };

    Ok(ConversationHeader {
        identity_public_key: root
            .get_identity_public_key()
            .map_err(|e| capnp_err(&e))?
            .to_vec(),
        profile,
        message_log_key: text_to_string(root.get_message_log_key().map_err(|e| capnp_err(&e))?)?,
        route_blob: bytes_or_empty(root.has_route_blob(), root.get_route_blob())?,
        prekey_bundle,
        created_at: root.get_created_at(),
        updated_at: root.get_updated_at(),
    })
}
