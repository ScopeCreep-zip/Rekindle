//! Navigation and activation keys that the Actions endpoint sends through the webview.

use super::key_input::map_special_key;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeKey {
    Tab,
    Enter,
    NumpadEnter,
    Escape,
    Space,
    Left,
    Right,
    Up,
    Down,
    ShiftLeft,
    ShiftRight,
}

impl NativeKey {
    pub(super) fn from_action(value: &str) -> Option<Self> {
        let code = map_special_key(value).map_or(value, |(_, code, _)| code);
        Some(match code {
            "Tab" => Self::Tab,
            "Enter" => Self::Enter,
            "NumpadEnter" => Self::NumpadEnter,
            "Escape" => Self::Escape,
            "Space" | " " => Self::Space,
            "ArrowLeft" => Self::Left,
            "ArrowRight" => Self::Right,
            "ArrowUp" => Self::Up,
            "ArrowDown" => Self::Down,
            "ShiftLeft" | "Shift" => Self::ShiftLeft,
            "ShiftRight" => Self::ShiftRight,
            _ => return None,
        })
    }
}

#[cfg(test)]
#[path = "native_key_tests.rs"]
mod tests;
