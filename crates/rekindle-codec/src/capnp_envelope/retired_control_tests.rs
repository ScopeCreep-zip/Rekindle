//! The control ordinals no peer ever sent (plan E3.2 S8-E: @19, @28, @30,
//! @34, @36–@38) are retired as `obsoleteN :Void`: a frame carrying one,
//! a forged thread archive among them, decodes to no payload, so nothing
//! reaches a receiver (threat model: unsigned frames never change state).

use super::{decode_community_envelope, pack, try_decode_community_envelope};
use crate::community_envelope_capnp::community_envelope;
use crate::community_envelope_capnp::control_payload;
use crate::error::CodecError;

type Retire = fn(control_payload::Builder<'_>);

const RETIRED: [(u16, Retire); 7] = [
    (19, |mut c| c.set_obsolete19(())),
    (28, |mut c| c.set_obsolete28(())),
    (30, |mut c| c.set_obsolete30(())),
    (34, |mut c| c.set_obsolete34(())),
    (36, |mut c| c.set_obsolete36(())),
    (37, |mut c| c.set_obsolete37(())),
    (38, |mut c| c.set_obsolete38(())),
];

fn frame(set: Retire) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    let root = builder.init_root::<community_envelope::Builder<'_>>();
    set(root.init_control());
    pack(&builder)
}

#[test]
fn a_retired_control_ordinal_decodes_to_no_payload() {
    for (ordinal, set) in RETIRED {
        let bytes = frame(set);
        assert!(
            matches!(
                decode_community_envelope(&bytes),
                Err(CodecError::UnknownVariant(_))
            ),
            "@{ordinal} decoded to a payload"
        );
        assert!(
            matches!(try_decode_community_envelope(&bytes), Ok(None)),
            "@{ordinal}: a relay must skip it, not fail the frame"
        );
    }
}
