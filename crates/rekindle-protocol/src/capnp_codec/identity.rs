
use super::{bytes_or_empty, capnp_err, pack, read_profile, unpack, write_profile, ProtocolError};
use crate::identity_capnp;

/// Domain struct for a user profile (used across account and conversation records).
#[derive(Debug, Clone)]
pub struct UserProfile {
    pub display_name: String,
    pub status_message: String,
    pub status: u8, // 0=online, 1=away, 2=busy, 3=offline
    pub avatar_hash: Vec<u8>,
    pub game_status: Option<crate::messaging::envelope::GameInfo>,
}

/// Domain struct for a Signal Protocol pre-key bundle.
///
/// Phase 3b of the decomposed-harvest plan augments this with PQXDH
/// fields. Old peers reading subkey 5 will fail to deserialize — that
/// is the intended hard break (pre-ship, no installed users).
#[derive(Debug, Clone)]
pub struct PreKeyBundle {
    // Classical X3DH layer.
    pub identity_key: Vec<u8>,
    pub signed_pre_key: Vec<u8>,
    pub signed_pre_key_sig: Vec<u8>,
    pub one_time_pre_key: Vec<u8>,
    pub one_time_pre_key_id: u32,
    pub registration_id: u32,
    // PQXDH layer.
    pub pqpk_lr: Vec<u8>,
    pub pqpk_lr_sig: Vec<u8>,
    pub pqpk_ot: Vec<u8>,
    pub pqpk_ot_sig: Vec<u8>,
    pub pqpk_ot_id: u32,
}

pub fn encode_profile(profile: &UserProfile) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    write_profile(
        builder.init_root::<identity_capnp::user_profile::Builder<'_>>(),
        profile,
    );
    pack(&builder)
}

pub fn decode_profile(data: &[u8]) -> Result<UserProfile, ProtocolError> {
    let reader = unpack(data)?;
    let root = reader
        .get_root::<identity_capnp::user_profile::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    read_profile(root)
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
        root.set_pqpk_lr(&bundle.pqpk_lr);
        root.set_pqpk_lr_sig(&bundle.pqpk_lr_sig);
        root.set_pqpk_ot(&bundle.pqpk_ot);
        root.set_pqpk_ot_sig(&bundle.pqpk_ot_sig);
        root.set_pqpk_ot_id(bundle.pqpk_ot_id);
        root.set_one_time_pre_key_id(bundle.one_time_pre_key_id);
    }
    pack(&builder)
}

pub fn decode_prekey_bundle(data: &[u8]) -> Result<PreKeyBundle, ProtocolError> {
    let reader = unpack(data)?;
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
        one_time_pre_key: bytes_or_empty(root.has_one_time_pre_key(), root.get_one_time_pre_key())?,
        one_time_pre_key_id: root.get_one_time_pre_key_id(),
        registration_id: root.get_registration_id(),
        pqpk_lr: root.get_pqpk_lr().map_err(|e| capnp_err(&e))?.to_vec(),
        pqpk_lr_sig: root.get_pqpk_lr_sig().map_err(|e| capnp_err(&e))?.to_vec(),
        pqpk_ot: bytes_or_empty(root.has_pqpk_ot(), root.get_pqpk_ot())?,
        pqpk_ot_sig: bytes_or_empty(root.has_pqpk_ot_sig(), root.get_pqpk_ot_sig())?,
        pqpk_ot_id: root.get_pqpk_ot_id(),
    })
}
