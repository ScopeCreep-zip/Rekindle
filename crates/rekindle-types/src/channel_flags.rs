/// Architecture §16.4 — voice message marker.
pub const VOICE_MESSAGE: u32 = 0x10;
/// Suppress OS push notification for this message.
pub const SUPPRESS_NOTIFICATIONS: u32 = 0x20;
/// Architecture §28.5 line 3111 — `mention_everyone` cleartext
/// signal. Receivers route notifications without decrypting the
/// body. Reader-validates: peers reject this bit from senders
/// without `MENTION_EVERYONE` permission (§9.3).
pub const MENTION_EVERYONE: u32 = 0x40;
/// Architecture §28.5 — `@here` (online-only) cleartext signal.
/// Same permission gate as `MENTION_EVERYONE`.
pub const MENTION_HERE: u32 = 0x80;
