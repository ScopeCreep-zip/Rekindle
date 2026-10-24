use super::direct::{data, fixed};
use super::{bounds, capnp_err, pack, unpack_bounded, CodecError};
use crate::identity_capnp;

/// Prekey record subkey 0 (plan E2.4): the signed prekey and the ML-KEM
/// last-resort key, each signed by the identity over `id ‖ key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrekeyRecordHead {
    pub spk_id: u32,
    pub spk: [u8; 32],
    pub spk_sig: [u8; 64],
    pub pq_lr_id: u32,
    /// ML-KEM-768 encapsulation key; its length is checked where it is
    /// parsed (`rekindle-crypto`).
    pub pq_lr: Vec<u8>,
    pub pq_lr_sig: [u8; 64],
    /// Unix ms the head was minted.
    pub issued_at: i64,
}

/// Prekey record subkeys 1..=100 (plan E2.4): one one-time curve prekey
/// and one one-time ML-KEM key, each signed by the identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpkSlot {
    pub opk_id: u32,
    pub opk: [u8; 32],
    pub opk_sig: [u8; 64],
    pub pq_opk_id: u32,
    pub pq_opk: Vec<u8>,
    pub pq_opk_sig: [u8; 64],
}

/// One device's prekey record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrekeyDevice {
    /// The identity's random `identity.device_id`.
    pub device_id: [u8; 16],
    /// The DFLT(101) record's key.
    pub record_key: String,
}

/// Profile subkey 5 (plan E2.4): each device's prekey record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrekeyRecordList {
    pub devices: Vec<PrekeyDevice>,
}

pub fn encode_prekey_head(head: &PrekeyRecordHead) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<identity_capnp::prekey_record_head::Builder<'_>>();
        root.set_spk_id(head.spk_id);
        root.set_spk(&head.spk);
        root.set_spk_sig(&head.spk_sig);
        root.set_pq_lr_id(head.pq_lr_id);
        root.set_pq_lr(&head.pq_lr);
        root.set_pq_lr_sig(&head.pq_lr_sig);
        root.set_issued_at(head.issued_at);
    }
    pack(&builder)
}

pub fn decode_prekey_head(bytes: &[u8]) -> Result<PrekeyRecordHead, CodecError> {
    let reader = unpack_bounded(bytes, bounds::PREKEY_MAX_WORDS)?;
    let r = reader
        .get_root::<identity_capnp::prekey_record_head::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    Ok(PrekeyRecordHead {
        spk_id: r.get_spk_id(),
        spk: fixed(r.get_spk(), "signed prekey")?,
        spk_sig: fixed(r.get_spk_sig(), "signed prekey signature")?,
        pq_lr_id: r.get_pq_lr_id(),
        pq_lr: data(r.get_pq_lr())?,
        pq_lr_sig: fixed(r.get_pq_lr_sig(), "last-resort signature")?,
        issued_at: r.get_issued_at(),
    })
}

pub fn encode_opk_slot(slot: &OpkSlot) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<identity_capnp::opk_slot::Builder<'_>>();
        root.set_opk_id(slot.opk_id);
        root.set_opk(&slot.opk);
        root.set_opk_sig(&slot.opk_sig);
        root.set_pq_opk_id(slot.pq_opk_id);
        root.set_pq_opk(&slot.pq_opk);
        root.set_pq_opk_sig(&slot.pq_opk_sig);
    }
    pack(&builder)
}

pub fn decode_opk_slot(bytes: &[u8]) -> Result<OpkSlot, CodecError> {
    let reader = unpack_bounded(bytes, bounds::PREKEY_MAX_WORDS)?;
    let r = reader
        .get_root::<identity_capnp::opk_slot::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    Ok(OpkSlot {
        opk_id: r.get_opk_id(),
        opk: fixed(r.get_opk(), "one-time prekey")?,
        opk_sig: fixed(r.get_opk_sig(), "one-time prekey signature")?,
        pq_opk_id: r.get_pq_opk_id(),
        pq_opk: data(r.get_pq_opk())?,
        pq_opk_sig: fixed(r.get_pq_opk_sig(), "one-time PQ prekey signature")?,
    })
}

pub fn encode_prekey_record_list(list: &PrekeyRecordList) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let root = builder.init_root::<identity_capnp::prekey_record_list::Builder<'_>>();
        let mut devices = root.init_devices(
            u32::try_from(list.devices.len()).expect("a device list fits a u32 index"),
        );
        for (i, device) in (0u32..).zip(&list.devices) {
            let mut entry = devices.reborrow().get(i);
            entry.set_device_id(&device.device_id);
            entry.set_record_key(&device.record_key);
        }
    }
    pack(&builder)
}

pub fn decode_prekey_record_list(bytes: &[u8]) -> Result<PrekeyRecordList, CodecError> {
    let reader = unpack_bounded(bytes, bounds::PREKEY_MAX_WORDS)?;
    let r = reader
        .get_root::<identity_capnp::prekey_record_list::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let devices = r
        .get_devices()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|d| {
            Ok(PrekeyDevice {
                device_id: fixed(d.get_device_id(), "device id")?,
                record_key: super::text_to_string(d.get_record_key().map_err(|e| capnp_err(&e))?)?,
            })
        })
        .collect::<Result<_, CodecError>>()?;
    Ok(PrekeyRecordList { devices })
}
