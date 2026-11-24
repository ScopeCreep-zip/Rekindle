//! A window's frame layout (plan SK0 S1, SK0-L1): the window kinds a skin
//! frames, the tiles that lay the frame and the pack's component tree. Re-exported from [`super::skin`], whose manifest carries them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::skin::Insets;

/// A window kind whose frame a skin lays (plan SK0 S1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkinWindow {
    /// The buddy list (Xfire's `<MainSkin>`).
    Main,
    /// A conversation window (Xfire's `<ChatSkin>`).
    Chat,
    /// The Community window, which Xfire never drew: composed from the
    /// pack's room window (`<GroupChatSkin>`), else its chat, else its buddy
    /// list (plan SK17).
    Community,
}

/// The pack component a window's layout was composed from (plan SK17), in
/// the Community window's source order. It becomes `WindowLayout.source`
/// when the importers write it and regenerate the built-ins (SK17-2/-3):
/// a committed built-in is the pipeline's fixed point, so a field it lacks
/// cannot be added without rewriting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutSource {
    /// The pack's room window (`<GroupChatSkin>`, INI `GroupChatSkin=`).
    Room,
    /// The pack's conversation window (`<ChatSkin>`).
    Chat,
    /// The pack's buddy list (`<MainSkin>`).
    Main,
}

/// A text role whose face a pack declares (plan SK17-4): the key of the
/// manifest's `text`, served as `--skin-text-<role>-*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextRole {
    /// A window's title lettering.
    Title,
    /// A list row (a buddy, a channel, a member).
    ListItem,
    /// A list's group or category header.
    ListHeader,
    /// The menu row.
    Menu,
    /// The composer and other text fields.
    Input,
    /// A sender's name in a conversation.
    ChatName,
}

/// The `manifest.icons` keys the importers fill for the Community window
/// (plan SK17-5): the one source of the names, never spelled again.
pub mod community_icon {
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
}

/// How a frame tile is placed along one axis of its box (plan SK0 S1):
/// the frame rectangle, or the whole window for an overhang tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub enum TileAxis {
    /// Once, its near side `offset` px from the start edge.
    Start { offset: u16 },
    /// Once, its far side `offset` px before the end edge.
    End { offset: u16 },
    /// Once, centred in the box.
    Centre,
    /// Repeated at its source size (never stretched) from `start` px to
    /// `end` px before the end edge, the last copy cut.
    Repeat { start: u16, end: u16 },
}

/// One frame tile: an image of the package placed by its anchors. Its
/// 1-bit mask (opaque where alpha ≥ 128) is derived from the image itself
/// wherever the layout is checked or shaped, so the shape has one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameTile {
    pub image: String,
    pub width: u16,
    pub height: u16,
    pub x: TileAxis,
    pub y: TileAxis,
    /// Placed in the whole window (art outside the main frame), not in
    /// the frame rectangle.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overhang: bool,
    /// A pack tile above the frame band (its Xfire `Z`): it may lie over the content rectangle, draws
    /// beneath the content's interactive layer, adds its opaque pixels outside the content to the
    /// silhouette and never cuts the content. `None` for a frame piece.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoration: Option<u8>,
    /// The pack's centre fill laid in the band between the frame pieces and the views (plan SK0-L1,
    /// lead decision): beside the content, or on its top or bottom line; a frame tile allowed only in
    /// that band, wholly opaque, never over the content.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fill: bool,
}

/// A window's frame layout (plan SK0 S1): the window is the frame
/// rectangle grown by `outsets` (room for overhang art); the content
/// rectangle is the frame rectangle shrunk by `content` and is always
/// opaque; `min_*` is the smallest frame the tiles fit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowLayout {
    pub outsets: Insets,
    pub content: Insets,
    pub min_width: u16,
    pub min_height: u16,
    pub tiles: Vec<FrameTile>,
    /// The pack-laid title strip (plan SK1-S1); `None` keeps the app's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<super::skin_title::TitleLayout>,
    /// The pack's component tree (plan SK2); `content` is its views node's box.
    /// The chat window's (plan SK17-2, `windows.chat.tree`) is served for
    /// XML packs only: an INI pack serves no chat tree until SK17-3, and no
    /// stand-in tree is made for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<super::skin_tree::SkinTree>,
    /// The popups the window's tiles launch (`LaunchSkin`), keyed by the
    /// pack's popup tile name (Symbiosis's `MainInfoViewPopup`,
    /// `Popups.xml:10`; StealthX's `MainMenuPopup`, `popups.xml:2`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub popups: BTreeMap<String, PopupLayout>,
}

/// The edge of the launching window a popup's left side is measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PopupEdge {
    Left,
    Right,
}

/// Where a popup's left side sits: `offset` px from the launching window's
/// `edge` (Symbiosis's `X="MainWindow.Right-13"` is `Right`, -13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PopupAnchor {
    pub edge: PopupEdge,
    pub offset: i16,
}

/// A popup a window launches (plan SK2 P1): its own box beside the window
/// and its component tree, laid by the same model as the window's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PopupLayout {
    pub anchor: PopupAnchor,
    /// The popup component's `DefaultWidth` (else its `MinWidth`), px.
    pub width: u16,
    pub min_width: u16,
    pub min_height: u16,
    /// Its vertical span in the launching window's height (Symbiosis's
    /// `ResizeY="100" IndentTop="23" IndentBottom="23"`).
    pub y: super::skin_tree::NodeSpan,
    pub tree: super::skin_tree::SkinTree,
}

impl PopupLayout {
    /// The popup's outline as `window`'s popup `name`, with its placement.
    #[must_use]
    pub fn shell(
        &self,
        window: SkinWindow,
        name: &str,
        min: (u16, u16),
        images: Vec<String>,
    ) -> super::skin_tree::SkinShell {
        super::skin_tree::SkinShell {
            popup: Some(super::skin_tree::ShellPopup {
                name: name.to_owned(),
                anchor: self.anchor,
                width: self.width,
                min_width: self.min_width,
                min_height: self.min_height,
                y: self.y,
            }),
            ..self.tree.shell(window, min, images)
        }
    }
}
