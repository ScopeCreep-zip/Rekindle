//! A skin package's manifest and the validator's typed refusal (plan
//! E7.7.1; `.claude/plans/skins/01-recommendation.md` §2).
//!
//! `rekindle-skin` parses `manifest.json` into [`SkinManifest`] and refuses
//! a package with a [`SkinError`]: the entry it refused and the rule it broke.
//! The manifest is a file format as much as a DTO, so unknown keys are
//! refused (`deny_unknown_fields`), never skipped.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The format string's prefix; the integer major follows it.
pub const SKIN_FORMAT_PREFIX: &str = "rekindle-skin/";

/// The one major version this build reads (`"rekindle-skin/1"`).
pub const SKIN_FORMAT_MAJOR: u32 = 1;

/// `manifest.json` at the package root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinManifest {
    /// `"rekindle-skin/<major>"`; an unknown major is refused.
    pub format: String,
    /// The skin's stable id: lowercase ASCII letters, digits, `-` and `.`.
    pub id: String,
    /// The display name: ≤ 64 scalars, NFC, no control or bidi characters.
    pub name: String,
    /// ≤ 280 scalars, NFC, no bidi characters, no control characters but a
    /// line feed.
    pub description: String,
    pub license: SkinLicense,
    /// The original format and its hash, for imported skins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkinSource>,
    /// The author's Ed25519 public key, hex (checked in E7.7.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// The author's Ed25519 signature, hex (checked in E7.7.6); needs `author`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// One or more variants, each validated alone.
    pub variants: Vec<SkinVariant>,
    /// Region specs keyed by registry region id.
    #[serde(default)]
    pub regions: BTreeMap<String, RegionSpec>,
    /// Icon entry paths keyed by semantic icon id.
    #[serde(default)]
    pub icons: BTreeMap<String, String>,
    /// Sound entry paths keyed by registry cue id.
    #[serde(default)]
    pub sounds: BTreeMap<String, String>,
}

/// A sound cue: the closed list of events a sound bank may voice (plan E5.2
/// "Sound cues", E7.7.4). It lives in this tier so E5.2's `notify_policy`
/// decides the same type a skin's bank is keyed by; "none" is the absence
/// of a cue, no variant. The serde name is the cue's id: the key of a
/// manifest's `sounds` and the stem of its `sounds/<id>.wav` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SoundCue {
    Message,
    Mention,
    SignOn,
    SignOff,
    Call,
    Ringback,
    Busy,
}

impl SoundCue {
    /// Every cue, in the plan's order.
    pub const ALL: [Self; 7] = [
        Self::Message,
        Self::Mention,
        Self::SignOn,
        Self::SignOff,
        Self::Call,
        Self::Ringback,
        Self::Busy,
    ];

    /// The cue's id (its serde name).
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Mention => "mention",
            Self::SignOn => "sign-on",
            Self::SignOff => "sign-off",
            Self::Call => "call",
            Self::Ringback => "ringback",
            Self::Busy => "busy",
        }
    }

    /// The cue whose id is `id`, if any.
    #[must_use]
    pub fn of(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }
}

/// A skin's licence: an SPDX expression or the licence's full text (legacy
/// packs carry restrictive terms no SPDX id names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkinLicense {
    Spdx(String),
    Text(String),
}

/// Where an imported skin came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinSource {
    /// The original format's id (e.g. `xfire-symbiosis`), same alphabet as
    /// [`SkinManifest::id`].
    pub format: String,
    /// BLAKE3 of the original archive, 64 lowercase hex digits.
    pub hash: String,
}

/// One variant: a base, a contrast level and its token file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinVariant {
    pub base: SkinBase,
    pub contrast: SkinContrast,
    /// A token file at the package root (`*.json`, not `manifest.json`).
    pub tokens: String,
}

/// Light or dark (VS Code `uiTheme`, Firefox `color_scheme`, Telegram
/// `base_theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkinBase {
    Light,
    Dark,
}

/// Standard or high contrast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkinContrast {
    Standard,
    High,
}

/// A design-token value once parsed to numbers (the restricted DTCG
/// profile); CSS is re-emitted from these, never from skin text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum TokenValue {
    /// sRGB, 8 bits a channel; `a` is 255 for `#RRGGBB`.
    Color { r: u8, g: u8, b: u8, a: u8 },
    /// Integer CSS pixels.
    Dimension(i32),
    /// Thousandths (≤ 3 decimals).
    Number(i64),
    /// A name from the bundled allow-list.
    FontFamily(String),
}

/// One region slot's bitmap skinning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionSpec {
    /// The normal-state image, an entry under `assets/`.
    pub image: String,
    pub fit: RegionFit,
    /// Per-state images, entries under `assets/`.
    #[serde(default)]
    pub states: BTreeMap<RegionState, String>,
    /// A registry token id tinting the image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<String>,
    /// An alpha mask, an entry under `assets/` (frame and titlebar only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
    /// The registry token id painted under text; required where the region
    /// carries text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_backing: Option<String>,
}

/// How a region image fills its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum RegionFit {
    Stretch,
    Tile,
    NineSlice(Insets),
}

/// Nine-slice insets in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Insets {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

/// The region states, Rekindle's own enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegionState {
    Normal,
    Hover,
    Down,
    Disabled,
    Focused,
    WindowInactive,
    Ringing,
    Talking,
    Muted,
}

/// Why a skin package was refused: the entry and the rule. Nothing is
/// partly applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("skin refused at {entry}: {rule}")]
pub struct SkinError {
    /// The zip entry name, `manifest.json`, or [`SkinError::PACKAGE`] for
    /// the archive as a whole.
    pub entry: String,
    pub rule: SkinRule,
}

impl SkinError {
    /// The `entry` of a refusal about the whole archive.
    pub const PACKAGE: &'static str = "<package>";

    #[must_use]
    pub fn new(entry: impl Into<String>, rule: SkinRule) -> Self {
        Self {
            entry: entry.into(),
            rule,
        }
    }
}

/// The rule a package broke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum SkinRule {
    #[error("package is {size} bytes, the limit is {max}")]
    PackageTooLarge { size: u64, max: u64 },
    #[error("not a well-formed zip: {detail}")]
    MalformedZip { detail: String },
    #[error("{count} entries, the limit is {max}")]
    TooManyEntries { count: u64, max: u64 },
    #[error("compressed entry (method {method}); only stored entries are read")]
    CompressedEntry { method: u16 },
    #[error("encrypted entry")]
    EncryptedEntry,
    #[error("symbolic link")]
    Symlink,
    #[error("not a regular file (mode {mode:o})")]
    SpecialFile { mode: u32 },
    #[error("absolute path")]
    AbsolutePath,
    #[error("`..` path component")]
    ParentTraversal,
    #[error("backslash in path")]
    Backslash,
    #[error("empty, `.` or control-character path component")]
    NonCanonicalPath,
    #[error("outside manifest.json, root token files, assets/, sounds/ and icons/")]
    OutsideLayout,
    #[error("duplicate name")]
    DuplicateName,
    #[error("name collides with {other} by case or normalisation")]
    CaseCollision { other: String },
    #[error("CRC-32 does not match the data")]
    CrcMismatch,
    #[error("manifest.json is missing")]
    MissingManifest,
    #[error("manifest is {size} bytes, the limit is {max}")]
    ManifestTooLarge { size: u64, max: u64 },
    #[error("manifest JSON: {detail}")]
    ManifestSyntax { detail: String },
    #[error("format major {major} is not supported")]
    UnsupportedFormat { major: u32 },
    #[error("{field}: expected {expected}")]
    BadValue { field: String, expected: String },
    #[error("{field} is empty")]
    EmptyText { field: String },
    #[error("{field} is {scalars} scalars, the limit is {max}")]
    TextTooLong {
        field: String,
        scalars: u64,
        max: u64,
    },
    #[error("{field} is not NFC")]
    NotNfc { field: String },
    #[error("{field} contains U+{codepoint:04X}")]
    ForbiddenCharacter { field: String, codepoint: u32 },
    #[error("license is not a valid SPDX expression: {detail}")]
    BadSpdx { detail: String },
    #[error("no variants")]
    NoVariants,
    #[error("two variants share base {base} and contrast {contrast}")]
    DuplicateVariant { base: String, contrast: String },
    #[error("{field} names an entry the package lacks")]
    MissingEntry { field: String },
    #[error("token file JSON: {detail}")]
    TokenSyntax { detail: String },
    #[error("{count} tokens, the limit is {max}")]
    TooManyTokens { count: u64, max: u64 },
    #[error("{token}: not a name of the closed token registry")]
    UnknownToken { token: String },
    #[error("{token}: a fixed token no skin sets")]
    FixedToken { token: String },
    #[error("{token}: `{key}` is not a key of the token profile")]
    TokenKey { token: String, key: String },
    #[error("{token}: the registry types it {expected}, the file {found}")]
    TokenTypeMismatch {
        token: String,
        expected: String,
        found: String,
    },
    #[error("{token}: expected {expected}")]
    BadTokenValue { token: String, expected: String },
    #[error("{token}: the alias chain returns to itself")]
    AliasCycle { token: String },
    #[error("{token}: the alias names {target}, which the file does not define")]
    UnresolvedAlias { token: String, target: String },
    #[error("expected {expected}")]
    UnsupportedEntryType { expected: String },
    #[error("{format} images are refused; skins carry PNG and GIF only")]
    RefusedImageFormat { format: String },
    #[error("declared {declared}, but the bytes are {found}")]
    ImageKindMismatch { declared: String, found: String },
    #[error("an animated PNG (APNG); a skin's animations are GIFs")]
    AnimatedPng,
    #[error("{width} × {height} px; at most {max} × {max}")]
    ImageSide { width: u32, height: u32, max: u32 },
    #[error("a frame is larger than {max} × {max} px")]
    ImageFrameSide { max: u32 },
    #[error("{pixels} pixels; at most {max}")]
    ImagePixels { pixels: u64, max: u64 },
    #[error("more than {max} frames")]
    TooManyFrames { max: u32 },
    #[error("decoding needs more than {max} bytes")]
    ImageAllocation { max: u64 },
    #[error("the images hold at least {pixels} pixels together; at most {max}")]
    ImagesTotalPixels { pixels: u64, max: u64 },
    #[error("image decode: {detail}")]
    ImageDecode { detail: String },
    #[error("image encode: {detail}")]
    ImageEncode { detail: String },
    #[error("{flashes} flashes in one second; at most {max} (WCAG 2.3.1)")]
    Flashing { flashes: u32, max: u32 },
    #[error("{flashes} red flashes in one second; at most {max} (WCAG 2.3.1)")]
    RedFlashing { flashes: u32, max: u32 },
    #[error("{count} sound cues, the limit is {max}")]
    TooManySoundCues { count: u64, max: u64 },
    #[error("icons.{icon}: not an icon slot of the registry")]
    UnknownIcon { icon: String },
    #[error("sounds.{cue}: not a sound cue of the registry")]
    UnknownSoundCue { cue: String },
    #[error("longer than {max_seconds} s")]
    SoundTooLong { max_seconds: u32 },
    #[error("sample rate {rate} Hz, at most {max}")]
    SampleRateTooHigh { rate: u32, max: u32 },
    #[error("{channels} channels, at most {max}")]
    TooManyChannels { channels: u32, max: u32 },
    #[error("sound decode: {detail}")]
    SoundDecode { detail: String },
    #[error("sound encode: {detail}")]
    SoundEncode { detail: String },
    #[error("{token} is translucent; text grounds and fills are opaque")]
    TranslucentGround { token: String },
    #[error(
        "{token}: luminance {luminance_x1000}/1000 is on the wrong side of 179/1000 for base {declared}"
    )]
    BaseMismatch {
        declared: String,
        token: String,
        luminance_x1000: u32,
    },
    #[error(
        "{rule}: {token} on {ground} is {measured_x100}/100:1, at least {required_x100}/100:1 is required"
    )]
    Contrast {
        rule: String,
        token: String,
        ground: String,
        measured_x100: u32,
        required_x100: u32,
    },
    #[error("{region}: not a region slot of the registry")]
    UnknownRegion { region: String },
    #[error("{region} carries text and needs a textBacking colour token")]
    MissingTextBacking { region: String },
    #[error("{region}: nine-slice insets {top}/{right}/{bottom}/{left} leave no middle in a {width} × {height} image")]
    NineSliceInsets {
        region: String,
        top: u16,
        right: u16,
        bottom: u16,
        left: u16,
        width: u32,
        height: u32,
    },
    #[error(
        "F83: {token} on {region} ({backing} over the image) in the {window} window at {width} × {height} px is {measured_x100}/100:1 at the worst pixel, at least {required_x100}/100:1 is required"
    )]
    TextOverBitmap {
        region: String,
        token: String,
        backing: String,
        window: String,
        width: u32,
        height: u32,
        measured_x100: u32,
        required_x100: u32,
    },
    #[error(
        "D1: danger.fill is OKLCH hue {hue_x10}/10°, chroma {chroma_x1000}/1000; danger is hue 12°–36°, chroma ≥ 0.06"
    )]
    DangerNotRed { hue_x10: u32, chroma_x1000: u32 },
    #[error(
        "D3: accent.fill is OKLCH hue {hue_x10}/10°, chroma {chroma_x1000}/1000, in the red (danger) category"
    )]
    AccentIsRed { hue_x10: u32, chroma_x1000: u32 },
    #[error(
        "{rule}: {a} and {b} are ΔE00 {measured_x10}/10 apart under {vision} vision, at least {required_x10}/10 is required"
    )]
    TooClose {
        rule: String,
        a: String,
        b: String,
        vision: String,
        measured_x10: u32,
        required_x10: u32,
    },
}
