// Nerd Font icon constants: codepoints in the shipped src/assets/fonts/SymbolsNerdFontMono-Regular.woff2
// (its name table: "Version 001.000;Nerd Fonts 3.3.0"), the ones the app shows as font glyphs.
// scripts/skin_glyphs.py draws a skinned window's icons as SVG (plan SK17-7a) from these and from its own
// codepoints for icons only a skinned window shows, and records each one's icon set and licence in
// src/assets/icons/NOTICE.md.

// Action bar
export const ICON_NEW_CHAT = "\u{F140F}";       // nf-md-chat_plus
export const ICON_ADD_FRIEND = "\u{F0014}";      // nf-md-account_plus
export const ICON_COMMUNITIES = "\u{F0849}";     // nf-md-account_group
export const ICON_LOGOUT = "\u{F0343}";          // nf-md-logout
export const ICON_KEYBOARD = "\u{F030C}";        // nf-md-keyboard

// Notifications
export const ICON_BELL = "\u{F009A}";            // nf-md-bell
export const ICON_CHECK = "\u{F012C}";           // nf-md-check

// Voice
export const ICON_HANGUP = "\u{F03F5}";          // nf-md-phone_hangup
export const ICON_PHONE = "\u{F03F2}";           // nf-md-phone
export const ICON_VOLUME_HIGH = "\u{F057E}";     // nf-md-volume_high
export const ICON_VIDEO = "\u{F0567}";            // nf-md-video

// Chat
export const ICON_SEND = "\u{F048A}";            // nf-md-send
export const ICON_ACCOUNT_REMOVE = "\u{F0015}";  // nf-md-account_remove

// Community
export const ICON_PLUS = "\u{F0415}";            // nf-md-plus — create community/group, new post (beside a label)
export const ICON_GAMEPAD = "\u{F0297}";          // nf-md-gamepad_variant
export const ICON_SHIELD = "\u{F088F}";           // nf-md-shield_account
export const ICON_KEY = "\u{F0306}";              // nf-md-key
