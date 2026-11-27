//! The device-key bodies (ADR 0022, plan E3.2 S7d-keys.1): `DeviceKeyEntry`
//! (an op arm, `MemberJoined`'s first device and the `GenesisCert`
//! creator's), `DeviceRetiredEntry`, `DeviceReadmitEntry` and
//! `DeviceReEnrolEntry` (S7d-keys.10).

use rekindle_types::governance::{
    DeviceEndorsement, DeviceReEnrol, DeviceReadmit, DeviceRetired, DeviceSetElement,
    DeviceSetElementKind, MAX_DEVICE_SET_ELEMENTS,
};
use rekindle_types::id::PseudonymKey;

use super::{data_field, fixed};
use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community_governance_capnp::{
    device_key_entry, device_re_enrol_entry, device_readmit_entry, device_retired_entry,
    DeviceSetElementKind as WireKind,
};
use crate::CodecError;

pub(crate) fn read_device_key(
    r: device_key_entry::Reader<'_>,
) -> Result<DeviceEndorsement, CodecError> {
    let list = r.get_extends_elements().map_err(|e| capnp_err(&e))?;
    if list.len() as usize > MAX_DEVICE_SET_ELEMENTS {
        return Err(CodecError::Deserialization(format!(
            "device endorsement extends {} elements, over {MAX_DEVICE_SET_ELEMENTS}",
            list.len()
        )));
    }
    let extends_elements = list
        .iter()
        .map(|e| {
            let kind = match e.get_kind().map_err(not_in_schema)? {
                WireKind::Endorsement => DeviceSetElementKind::Endorsement,
                WireKind::Retirement => DeviceSetElementKind::Retirement,
            };
            let key = fixed(data_field(e.get_device_key())?, "device set element key")?;
            Ok(DeviceSetElement::new(kind, key, e.get_incarnation()))
        })
        .collect::<Result<Vec<_>, CodecError>>()?;
    Ok(DeviceEndorsement {
        device_key: fixed(data_field(r.get_device_key())?, "device key")?,
        incarnation: r.get_incarnation(),
        extends_set: fixed(data_field(r.get_extends_set())?, "device extendsSet")?,
        extends_elements,
        device_record: text_to_string(r.get_device_record().map_err(|e| capnp_err(&e))?)?,
        device_record_owner: text_to_string(
            r.get_device_record_owner().map_err(|e| capnp_err(&e))?,
        )?,
        fork_evidence: fixed(data_field(r.get_fork_evidence())?, "device forkEvidence")?,
        endorsement_sig: data_field(r.get_endorsement_sig())?.to_vec(),
    })
}

pub(crate) fn write_device_key(mut b: device_key_entry::Builder<'_>, e: &DeviceEndorsement) {
    b.set_device_key(&e.device_key);
    b.set_incarnation(e.incarnation);
    b.set_extends_set(&e.extends_set);
    let n = u32::try_from(e.extends_elements.len()).unwrap_or(u32::MAX);
    let mut list = b.reborrow().init_extends_elements(n);
    for (i, el) in (0..n).zip(&e.extends_elements) {
        let mut w = list.reborrow().get(i);
        w.set_kind(match el.kind {
            DeviceSetElementKind::Endorsement => WireKind::Endorsement,
            DeviceSetElementKind::Retirement => WireKind::Retirement,
        });
        w.set_device_key(&el.device_key);
        w.set_incarnation(el.incarnation);
    }
    b.set_device_record(e.device_record.as_str());
    b.set_device_record_owner(e.device_record_owner.as_str());
    b.set_fork_evidence(&e.fork_evidence);
    b.set_endorsement_sig(&e.endorsement_sig);
}

pub(crate) fn read_device_retired(
    r: device_retired_entry::Reader<'_>,
) -> Result<DeviceRetired, CodecError> {
    Ok(DeviceRetired {
        device_key: fixed(data_field(r.get_device_key())?, "retired device key")?,
        incarnation: r.get_incarnation(),
        last_seq: r.get_last_seq(),
        retirement_sig: data_field(r.get_retirement_sig())?.to_vec(),
    })
}

pub(crate) fn write_device_retired(mut b: device_retired_entry::Builder<'_>, r: &DeviceRetired) {
    b.set_device_key(&r.device_key);
    b.set_incarnation(r.incarnation);
    b.set_last_seq(r.last_seq);
    b.set_retirement_sig(&r.retirement_sig);
}

pub(crate) fn read_device_readmit(
    r: device_readmit_entry::Reader<'_>,
) -> Result<DeviceReadmit, CodecError> {
    Ok(DeviceReadmit {
        member: PseudonymKey(fixed(data_field(r.get_member())?, "readmit member")?),
        member_record: text_to_string(r.get_member_record().map_err(|e| capnp_err(&e))?)?,
        new_key: fixed(data_field(r.get_new_key())?, "readmit new key")?,
        incarnation: r.get_incarnation(),
        fork_evidence: fixed(data_field(r.get_fork_evidence())?, "readmit forkEvidence")?,
    })
}

pub(crate) fn write_device_readmit(mut b: device_readmit_entry::Builder<'_>, r: &DeviceReadmit) {
    b.set_member(&r.member.0);
    b.set_member_record(r.member_record.as_str());
    b.set_new_key(&r.new_key);
    b.set_incarnation(r.incarnation);
    b.set_fork_evidence(&r.fork_evidence);
}

pub(crate) fn read_device_re_enrol(
    r: device_re_enrol_entry::Reader<'_>,
) -> Result<DeviceReEnrol, CodecError> {
    Ok(DeviceReEnrol {
        retired: read_device_retired(r.get_retired().map_err(|e| capnp_err(&e))?)?,
        endorsement: read_device_key(r.get_endorsement().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(crate) fn write_device_re_enrol(mut b: device_re_enrol_entry::Builder<'_>, r: &DeviceReEnrol) {
    write_device_retired(b.reborrow().init_retired(), &r.retired);
    write_device_key(b.init_endorsement(), &r.endorsement);
}
