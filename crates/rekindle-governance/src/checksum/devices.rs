//! The device-set section of `canonical_state` and its decoder (plan E3.2
//! K-CP, ADR 0022 rules 2, 5–7): each member's endorsements, retirements,
//! fork evidence and readmits, so a checkpoint's `stateRoot` commits to the
//! set a resuming reader judges device keys against. Every struct is
//! destructured exhaustively, so a new field cannot compile without its
//! encoding.

use rekindle_types::governance::{DeviceEndorsement, DeviceSetElement, DeviceSetElementKind};

use super::canon::{put_len, Canon};
use super::decode::{CanonReader, Decanon};
use crate::device_judge::{DeviceView, MemberDevices};

impl Canon for DeviceView {
    fn put(&self, out: &mut Vec<u8>) {
        let Self { members } = self;
        members.put(out);
    }
}

impl Decanon for DeviceView {
    fn take(r: &mut CanonReader<'_>) -> Option<Self> {
        Some(Self {
            members: Decanon::take(r)?,
        })
    }
}

impl Canon for MemberDevices {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            member_record,
            endorsements,
            retirements,
            forks,
            readmits,
        } = self;
        member_record.put(out);
        endorsements.put(out);
        retirements.put(out);
        forks.put(out);
        readmits.put(out);
    }
}

impl Decanon for MemberDevices {
    fn take(r: &mut CanonReader<'_>) -> Option<Self> {
        Some(Self {
            member_record: Decanon::take(r)?,
            endorsements: Decanon::take(r)?,
            retirements: Decanon::take(r)?,
            forks: Decanon::take(r)?,
            readmits: Decanon::take(r)?,
        })
    }
}

impl Canon for DeviceEndorsement {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            device_key,
            incarnation,
            extends_set,
            extends_elements,
            device_record,
            device_record_owner,
            fork_evidence,
            endorsement_sig,
        } = self;
        device_key.put(out);
        incarnation.put(out);
        extends_set.put(out);
        extends_elements.put(out);
        device_record.put(out);
        device_record_owner.put(out);
        fork_evidence.put(out);
        put_len(out, endorsement_sig.len());
        out.extend_from_slice(endorsement_sig);
    }
}

impl Decanon for DeviceEndorsement {
    fn take(r: &mut CanonReader<'_>) -> Option<Self> {
        Some(Self {
            device_key: Decanon::take(r)?,
            incarnation: Decanon::take(r)?,
            extends_set: Decanon::take(r)?,
            extends_elements: Decanon::take(r)?,
            device_record: Decanon::take(r)?,
            device_record_owner: Decanon::take(r)?,
            fork_evidence: Decanon::take(r)?,
            endorsement_sig: {
                let n = r.len()?;
                r.take(n)?.to_vec()
            },
        })
    }
}

impl Canon for DeviceSetElement {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            kind,
            device_key,
            incarnation,
        } = self;
        out.push(match kind {
            DeviceSetElementKind::Endorsement => 0,
            DeviceSetElementKind::Retirement => 1,
        });
        device_key.put(out);
        incarnation.put(out);
    }
}

impl Decanon for DeviceSetElement {
    fn take(r: &mut CanonReader<'_>) -> Option<Self> {
        let kind = match r.take(1)? {
            [0] => DeviceSetElementKind::Endorsement,
            [1] => DeviceSetElementKind::Retirement,
            _ => return None,
        };
        Some(Self {
            kind,
            device_key: Decanon::take(r)?,
            incarnation: Decanon::take(r)?,
        })
    }
}
