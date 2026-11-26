/// A voice channel (Xfire's `VoiceServer` icon).
pub const VOICE_SERVER: &str = "voice.server";
/// The microphone open.
pub const MIC_ON: &str = "mic.on";
/// The microphone muted.
pub const MIC_OFF: &str = "mic.off";
/// The speaker on.
pub const SPEAKER_ON: &str = "speaker.on";
/// The speaker muted.
pub const SPEAKER_MUTED: &str = "speaker.muted";
/// The speaker off (deafened).
pub const SPEAKER_OFF: &str = "speaker.off";
/// A category's fold arrow, open.
pub const FOLD_OPEN: &str = "fold.open";
/// A category's fold arrow, closed.
pub const FOLD_CLOSED: &str = "fold.closed";
/// A room (the community itself).
pub const ROOM: &str = "room";
/// A text channel.
pub const CHAT: &str = "chat";
/// Every name above, in plan order.
pub const ALL: [&str; 10] = [
    VOICE_SERVER,
    MIC_ON,
    MIC_OFF,
    SPEAKER_ON,
    SPEAKER_MUTED,
    SPEAKER_OFF,
    FOLD_OPEN,
    FOLD_CLOSED,
    ROOM,
    CHAT,
];
