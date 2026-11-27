use super::*;
use crate::capnp_codec::direct::{encode_root, DirectBody, DmTypeId};
use rekindle_types::disappearing::MAX_SECONDS;

#[test]
fn a_timer_past_four_weeks_refuses_the_frame() {
    let bytes = |seconds| encode_root::<disappearing_timer::Owned>(|mut b| b.set_seconds(seconds));
    let ok = DirectBody::decode(DmTypeId::DisappearingTimer, &bytes(MAX_SECONDS).unwrap()).unwrap();
    assert_eq!(
        ok,
        DirectBody::DisappearingTimer(DisappearingTimer {
            seconds: MAX_SECONDS,
            group_id: None,
        })
    );
    assert!(DirectBody::decode(DmTypeId::DisappearingTimer, &bytes(MAX_SECONDS + 1).unwrap()).is_err());
    let text = encode_root::<dm_text::Owned>(|mut b| {
        b.set_client_id(&[1; 16]);
        b.set_body("hi");
        b.set_expire_seconds(MAX_SECONDS + 1);
    });
    assert!(DirectBody::decode(DmTypeId::DmText, &text.unwrap()).is_err());
}
