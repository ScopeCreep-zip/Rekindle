use std::cell::Cell;

use super::{AudioDeviceKind, AudioPrefs, AudioTargets, DeviceDirection, VoiceError};
use crate::{AudioIo, VoiceSink, VoiceSource};

#[test]
fn headless_io_selects_no_device() {
    let mut prefs = AudioPrefs {
        noise_suppression: false,
        echo_cancellation: false,
        input_volume: 1.0,
        output_volume: 1.0,
        input_device: Some("no-such-input".into()),
        output_device: Some("no-such-output".into()),
        input_channels: std::collections::BTreeMap::new(),
        io: AudioIo::HEADLESS,
    };
    let queried = Cell::new(0);
    let missing = |_: Option<&str>, _: &DeviceDirection| {
        queried.set(queried.get() + 1);
        Err(VoiceError::NoAudioDevice(AudioDeviceKind::Input))
    };
    let headless = AudioTargets::select_with(&prefs, missing);
    assert_eq!(queried.get(), 0);
    assert!(headless.unwrap().match_open(&(None, None)));

    prefs.io = AudioIo {
        source: VoiceSource::Mic,
        sink: VoiceSink::Null,
    };
    let microphone = AudioTargets::select_with(&prefs, missing);
    assert_eq!(queried.get(), 1);
    assert!(matches!(
        microphone,
        Err(VoiceError::NoAudioDevice(AudioDeviceKind::Input))
    ));
    assert_eq!(
        microphone.unwrap_err().notice(),
        "No microphone is available."
    );
}
