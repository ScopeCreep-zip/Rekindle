//! The engine's audio ends without a device (plan E4.1 `AudioIo`).

use super::*;

/// Under `Silence`/`Null` a session's audio comes up with no device: the
/// configured device names exist nowhere, so opening either would fail.
/// Silence delivers no PCM and Null takes more than its channel holds.
#[test]
fn a_headless_engine_runs_with_no_audio_device() {
    let mut engine = VoiceEngine::new(
        VoiceConfig {
            io: AudioIo::HEADLESS,
            input_device: Some("no-such-input".into()),
            output_device: Some("no-such-output".into()),
            ..VoiceConfig::default()
        },
        std::sync::Arc::new(rekindle_utils::SystemClock),
    )
    .unwrap();
    engine.start_capture().unwrap();
    engine.start_playback().unwrap();
    assert!(engine.capture.as_ref().is_some_and(AudioCapture::is_active));
    assert!(engine
        .playback
        .as_ref()
        .is_some_and(AudioPlayback::is_active));

    let capture = engine.take_capture_rx().unwrap();
    let playback = engine.take_playback_tx().unwrap();
    for _ in 0..300 {
        playback.blocking_send(vec![0.0; 960]).unwrap();
    }
    assert!(capture.is_empty());

    engine.stop_capture();
    engine.stop_playback();
    assert!(engine.capture.is_none() && engine.playback.is_none());
}
