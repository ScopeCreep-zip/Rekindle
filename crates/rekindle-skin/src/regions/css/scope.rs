//! The windows a pack's art paints (plan E7.7). Xfire's packs skin only
//! their own components: Symbiosis's `Components/` holds `MainWindow`,
//! `ChatWindow`, `GroupChatWindow`, the popups (`MainInfo`, `ChatInfo`) and
//! `Common` (shared images, no window). Tools → Options was never skinned
//! (Symbiosis's `Xig2Options` is the in-game overlay's), so Settings, and
//! every window Rekindle adds that Xfire never drew but the Community
//! window, keep the theme's tokens and none of the pack's art.
//!
//! Every window's root `.app-frame` carries `data-window`, its window kind
//! in kebab case (`login`, `buddy-list`, `settings`, `skin-editor`, `chat`,
//! `group-chat`, `community`, `community-browser`, `call`, `profile`,
//! `popup`). The skin layer's region, band and icon properties are set on
//! the roots of the skinned kinds, so they inherit only inside those
//! windows; elsewhere each `var()` is unset and the app's own rule shows.

/// Each pack component that is a window and the app window kinds it skins.
/// The Community window is composed from the pack's room window
/// (`GroupChatWindow`), else its chat, else its buddy list (plan SK17,
/// `SkinWindow::Community`), so it is the room window's kind.
pub const PACK_WINDOWS: [(&str, &[&str]); 4] = [
    ("MainWindow", &["buddy-list"]),
    ("ChatWindow", &["chat"]),
    ("GroupChatWindow", &["group-chat", "community"]),
    ("Popups", &["popup"]),
];

/// The window kinds no pack component draws: theme tokens only.
pub const UNSKINNED_WINDOWS: [&str; 6] = [
    "login",
    "settings",
    "skin-editor",
    "community-browser",
    "call",
    "profile",
];

/// The window kinds the pack's art paints, in [`PACK_WINDOWS`] order.
pub fn skinned_windows() -> impl Iterator<Item = &'static str> {
    PACK_WINDOWS
        .iter()
        .flat_map(|(_, kinds)| kinds.iter().copied())
}

/// The selector of every skinned window's root:
/// `.app-frame:is([data-window="buddy-list"], …)`.
#[must_use]
pub fn skinned_root() -> String {
    let kinds: Vec<String> = skinned_windows()
        .map(|k| format!("[data-window=\"{k}\"]"))
        .collect();
    format!(".app-frame:is({})", kinds.join(", "))
}

/// `decls` in the skin layer on the skinned windows' roots.
#[must_use]
pub fn skin_layer(decls: &str) -> String {
    format!("@layer skin {{\n  {} {{\n{decls}  }}\n}}\n", skinned_root())
}
