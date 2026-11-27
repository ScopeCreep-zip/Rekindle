use super::{voice_channel_or, DEAFEN_NEEDS_VOICE, MUTE_NEEDS_VOICE};

/// A server mute or deafen with no voice channel answers its words.
#[test]
fn a_null_channel_returns_the_refusal_text() {
    assert_eq!(
        voice_channel_or(None, MUTE_NEEDS_VOICE),
        Err("Mute is for members in a voice channel.".to_owned())
    );
    assert_eq!(
        voice_channel_or(None, DEAFEN_NEEDS_VOICE),
        Err("Deafen is for members in a voice channel.".to_owned())
    );
    assert_eq!(
        voice_channel_or(Some("c".into()), MUTE_NEEDS_VOICE),
        Ok("c".to_owned())
    );
}
