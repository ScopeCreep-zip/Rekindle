use rekindle_types::presence::{LastSeenPrecision, PresencePolicyField, ShareScope};

use super::SettingsRequest;
use crate::framing::{decode_frame, encode_frame};
use crate::protocol::{CommunityRequest, IpcRequest, Lane};

#[test]
fn location_policy_requests_round_trip_on_the_bus() {
    let read = IpcRequest::Community(CommunityRequest::Settings(
        SettingsRequest::CommunityGetPresencePolicy {
            community: "group".into(),
        },
    ));
    assert!(matches!(
        decode_frame::<IpcRequest>(&encode_frame(&read).unwrap()).unwrap(),
        IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunityGetPresencePolicy { community }
        )) if community == "group"
    ));
    assert_eq!(read.lane(), Lane::Query);

    for field in [
        PresencePolicyField::ShareOnline(true),
        PresencePolicyField::ShareLocation(ShareScope::Members),
        PresencePolicyField::ShareActivity(ShareScope::None),
        PresencePolicyField::LastSeen(LastSeenPrecision::Exact),
    ] {
        let write = IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunitySetPresencePolicyField {
                community: "group".into(),
                field,
            },
        ));
        assert!(matches!(
            decode_frame::<IpcRequest>(&encode_frame(&write).unwrap()).unwrap(),
            IpcRequest::Community(CommunityRequest::Settings(
                SettingsRequest::CommunitySetPresencePolicyField { community, field: actual }
            )) if community == "group" && actual == field
        ));
        assert_eq!(write.lane(), Lane::Write);
    }
}
