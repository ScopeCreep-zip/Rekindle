//! Action string → KeymapAction enum mapping.

use crate::v2::tui::state::search::SearchMode;

/// Actions the keymap system can produce. Consumed by process/input.rs.
#[derive(Clone, Debug)]
pub enum KeymapAction {
    Quit,
    ToggleHelp,
    FocusNext,
    FocusPrev,
    ScrollDown(u16),
    ScrollUp(u16),
    ScrollToBottom,
    ScrollToTop,
    ScrollPageDown,
    ScrollPageUp,
    Select,
    Cancel,
    Back,
    NextTab,
    PrevTab,
    OpenSearch(SearchMode),
    OpenQuickSwitcher,
    Refresh,
    ToggleSidebar,
    EnterInputMode,
    ExitInputMode,
    InputSubmit,
    ShowDashboard,
    ShowFriendList,
    ShowDmInbox,
    ShowDoctor,
    ShowIdentitySettings,
    ReplyToSelected,
    EditSelected,
    CloseSplitDm,
    OpenFileContentSearch,
    ToggleTimezone,
}

/// Map an action string from the JSON keymap to a KeymapAction enum variant.
/// Exhaustive — every action string the keymap can produce must be here.
pub fn map_action(action: &str) -> Option<KeymapAction> {
    match action {
        "Quit" => Some(KeymapAction::Quit),
        "ToggleHelp" => Some(KeymapAction::ToggleHelp),
        "FocusNext" => Some(KeymapAction::FocusNext),
        "FocusPrev" => Some(KeymapAction::FocusPrev),
        "ScrollDown" => Some(KeymapAction::ScrollDown(1)),
        "ScrollUp" => Some(KeymapAction::ScrollUp(1)),
        "ScrollToBottom" => Some(KeymapAction::ScrollToBottom),
        "ScrollToTop" => Some(KeymapAction::ScrollToTop),
        "ScrollPageDown" => Some(KeymapAction::ScrollPageDown),
        "ScrollPageUp" => Some(KeymapAction::ScrollPageUp),
        "Select" => Some(KeymapAction::Select),
        "Cancel" => Some(KeymapAction::Cancel),
        "Back" => Some(KeymapAction::Back),
        "NextTab" => Some(KeymapAction::NextTab),
        "PrevTab" => Some(KeymapAction::PrevTab),
        "OpenSearch" => Some(KeymapAction::OpenSearch(SearchMode::MessageSearch)),
        "OpenQuickSwitcher" => Some(KeymapAction::OpenQuickSwitcher),
        "Refresh" => Some(KeymapAction::Refresh),
        "ToggleSidebar" => Some(KeymapAction::ToggleSidebar),
        "EnterInputMode" => Some(KeymapAction::EnterInputMode),
        "ExitInputMode" => Some(KeymapAction::ExitInputMode),
        "InputSubmit" => Some(KeymapAction::InputSubmit),
        "OpenCommandPalette" => Some(KeymapAction::OpenSearch(SearchMode::CommandPalette)),
        "ShowDashboard" => Some(KeymapAction::ShowDashboard),
        "ShowFriendList" => Some(KeymapAction::ShowFriendList),
        "ShowDmInbox" => Some(KeymapAction::ShowDmInbox),
        "ShowDoctor" => Some(KeymapAction::ShowDoctor),
        "ShowIdentitySettings" => Some(KeymapAction::ShowIdentitySettings),
        "ReplyToSelected" => Some(KeymapAction::ReplyToSelected),
        "EditSelected" => Some(KeymapAction::EditSelected),
        "CloseSplitDm" => Some(KeymapAction::CloseSplitDm),
        "OpenFileContentSearch" => Some(KeymapAction::OpenFileContentSearch),
        "ToggleTimezone" => Some(KeymapAction::ToggleTimezone),
        unknown => {
            tracing::warn!(action = unknown, "unknown keymap action — ignoring");
            None
        }
    }
}
