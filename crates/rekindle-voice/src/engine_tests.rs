use tokio::sync::mpsc::error::TryRecvError;

use super::{AudioIo, AudioPrefs, VoiceEngine};

#[test]
fn a_headless_engine_runs_with_no_audio_device() {
    let mut engine = VoiceEngine::open(
        &AudioPrefs {
            io: AudioIo::HEADLESS,
            noise_suppression: true,
            echo_cancellation: true,
            input_volume: 1.0,
            output_volume: 1.0,
            input_device: Some("no-such-input".into()),
            output_device: Some("no-such-output".into()),
            input_channels: std::collections::BTreeMap::new(),
        },
        std::sync::Arc::new(rekindle_utils::SystemClock),
    );
    assert!(engine.is_ok(), "Silence/Null must open without a device");
    let engine = engine.as_mut().unwrap();
    let mut capture = engine.take_capture_rx().unwrap();
    assert_eq!(capture.try_recv(), Err(TryRecvError::Empty));
    let playback = engine.take_playback_output();
    assert!(
        playback.is_some(),
        "the receive loop needs a playback endpoint"
    );
    let playback = playback.unwrap();
    for _ in 0..300 {
        assert!(playback.play(vec![0.0; 960]));
    }

    engine.stop_capture();
    engine.stop_playback();
    assert_eq!(capture.try_recv(), Err(TryRecvError::Disconnected));
    assert!(engine.take_playback_output().is_none());
}
