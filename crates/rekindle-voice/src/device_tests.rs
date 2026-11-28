use super::{choose_device, AudioDeviceKind, VoiceError};

fn no_device() -> Result<(String, String), VoiceError> {
    Err(VoiceError::NoAudioDevice(AudioDeviceKind::Input))
}

/// A connected saved device is opened as chosen.
#[test]
fn a_connected_saved_device_is_opened() {
    let picked = choose_device(Some("usb"), Some("USB Mic".into()), no_device).unwrap();
    assert_eq!(picked.id, "usb");
    assert_eq!(picked.saved_missing, None);
}

/// Under `Mic`, a saved device that is gone keeps the C7.24 state: the
/// system default stands in and the saved choice is kept to announce
/// it; with no device at all the session gets the error, never
/// `Silence` in its place.
#[test]
fn a_missing_device_stays_the_visible_c7_24_state() {
    let default = || Ok(("builtin".to_string(), "Built-in".to_string()));
    let stand_in = choose_device(Some("usb"), None, default).unwrap();
    assert_eq!(stand_in.id, "builtin");
    assert_eq!(stand_in.saved_missing.as_deref(), Some("usb"));
    assert!(matches!(
        choose_device(Some("usb"), None, no_device),
        Err(VoiceError::NoAudioDevice(AudioDeviceKind::Input))
    ));
    assert!(matches!(
        choose_device(None, None, no_device),
        Err(VoiceError::NoAudioDevice(AudioDeviceKind::Input))
    ));
}
