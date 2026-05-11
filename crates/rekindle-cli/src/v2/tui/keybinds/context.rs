//! Keymap context — determines which bindings are active.

/// Context in which a keybinding is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum KeymapContext {
    /// Normal navigation mode.
    Default,
    /// Text input mode — keys go to the input box.
    Input,
    /// Search overlay is active.
    Search,
    /// A modal overlay is active (help, confirm, etc.).
    Overlay,
}
