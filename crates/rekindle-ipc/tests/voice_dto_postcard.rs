//! `dto::voice` over the daemon IPC codec (`rekindle_ipc::framing::
//! {encode_frame, decode_frame}`, postcard), plan E4.1 S5a1: every
//! `VoiceEndState` variant and every `VoiceScope` variant round-trips.

use rekindle_ipc::framing::{decode_frame, encode_frame};
use rekindle_types::dto::voice::{VoiceDevice, VoiceDevices, VoiceEndState, VoiceStatus};
use rekindle_types::subscription_events::VoiceScope;

fn round_trip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    decode_frame(&encode_frame(value).expect("encodes")).expect("decodes")
}

fn device(stands_in_for: Option<&str>) -> VoiceEndState {
    VoiceEndState::Device {
        id: "builtin".into(),
        name: "Built-in".into(),
        stands_in_for: stands_in_for.map(Into::into),
    }
}

#[test]
fn voice_status_round_trips_over_the_ipc_codec() {
    let ends = [
        device(None),
        device(Some("usb")),
        VoiceEndState::Unavailable {
            reason: "no input device available".into(),
        },
        VoiceEndState::Headless,
    ];
    let scopes = [
        None,
        Some(VoiceScope::Community {
            community: "c".into(),
            channel: "v".into(),
        }),
        Some(VoiceScope::Dm {
            peer_key: "p".into(),
        }),
        Some(VoiceScope::Group {
            group_id: "g".into(),
        }),
    ];
    for input in &ends {
        for output in &ends {
            for session in &scopes {
                let status = VoiceStatus {
                    session: session.clone(),
                    start_refusal: session
                        .as_ref()
                        .map(|_| "Leave the current call first.".into()),
                    muted: true,
                    deafened: false,
                    input: input.clone(),
                    output: output.clone(),
                };
                assert_eq!(round_trip(&status), status);
            }
        }
    }
}

#[test]
fn voice_devices_round_trip_over_the_ipc_codec() {
    let devices = VoiceDevices {
        input_devices: vec![VoiceDevice {
            id: "mic".into(),
            name: "Mic".into(),
            is_default: true,
            channels: 2,
        }],
        output_devices: vec![VoiceDevice {
            id: "spk".into(),
            name: "Speakers".into(),
            is_default: false,
            channels: 0,
        }],
    };
    assert_eq!(round_trip(&devices), devices);
}
