/// A whole package.
pub const MAX_PACKAGE_BYTES: u64 = 16 * 1024 * 1024;
/// Zip entries, directories included.
pub const MAX_ENTRIES: u64 = 512;
/// `manifest.json`.
pub const MAX_MANIFEST_BYTES: u64 = 128 * 1024;
/// An image's longest side, px (≤ 2048²).
pub const MAX_IMAGE_SIDE: u32 = 2048;
/// An image's pixels (≤ 4 Mpx).
pub const MAX_IMAGE_PIXELS: u64 = 4_000_000;
/// An animated GIF's frames.
pub const MAX_IMAGE_FRAMES: u32 = 128;
/// Every image's pixels together, each animation frame counted
/// (≤ 64 Mpx total).
pub const MAX_TOTAL_IMAGE_PIXELS: u64 = 64_000_000;
/// What decoding one still image may hold: a 4 Mpx RGBA16 buffer
/// (32 MB) and the decoder's own state.
pub const MAX_IMAGE_ALLOC: u64 = 64 * 1024 * 1024;
/// Sound cues (files under `sounds/`).
pub const MAX_SOUND_CUES: u64 = 64;
/// A cue's length, seconds.
pub const MAX_SOUND_SECONDS: u32 = 5;
/// A cue's sample rate, Hz.
pub const MAX_SAMPLE_RATE: u32 = 48_000;
/// A cue's channels.
pub const MAX_SOUND_CHANNELS: u32 = 2;
