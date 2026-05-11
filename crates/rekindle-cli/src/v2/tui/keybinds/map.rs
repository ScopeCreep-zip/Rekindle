//! Action string → Action enum mapping.

use super::super::action::{Action, SearchMode};

/// Map an action string from the JSON keymap to an Action enum variant.
/// Exhaustive — every action string the keymap can produce must be here.
pub fn map_action(action: &str) -> Option<Action> {
    match action {
        "Quit" => Some(Action::Quit),
        "ToggleHelp" => Some(Action::ToggleHelp),
        "FocusNext" => Some(Action::FocusNext),
        "FocusPrev" => Some(Action::FocusPrev),
        "ScrollDown" => Some(Action::ScrollDown(1)),
        "ScrollUp" => Some(Action::ScrollUp(1)),
        "ScrollToBottom" => Some(Action::ScrollToBottom),
        "ScrollToTop" => Some(Action::ScrollToTop),
        "ScrollPageDown" => Some(Action::ScrollPageDown),
        "ScrollPageUp" => Some(Action::ScrollPageUp),
        "Select" => Some(Action::Select),
        "Cancel" => Some(Action::Cancel),
        "Back" => Some(Action::Back),
        "NextTab" => Some(Action::NextTab),
        "PrevTab" => Some(Action::PrevTab),
        "OpenSearch" => Some(Action::OpenSearch(SearchMode::MessageSearch)),
        "OpenQuickSwitcher" => Some(Action::OpenQuickSwitcher),
        "Refresh" => Some(Action::Refresh),
        "ToggleSidebar" => Some(Action::ToggleSidebar),
        "EnterInputMode" => Some(Action::EnterInputMode),
        "ExitInputMode" => Some(Action::ExitInputMode),
        "InputSubmit" => Some(Action::InputSubmit),
        "OpenCommandPalette" => Some(Action::OpenSearch(SearchMode::CommandPalette)),
        "ShowDashboard" => Some(Action::ShowDashboard),
        "ShowFriendList" => Some(Action::ShowFriendList),
        "ShowDmInbox" => Some(Action::ShowDmInbox),
        "ReplyToSelected" => Some(Action::ReplyToSelected),
        "EditSelected" => Some(Action::EditSelected),
        "CloseSplitDm" => Some(Action::CloseSplitDm),
        "OpenFileContentSearch" => Some(Action::OpenFileContentSearch),
        unknown => {
            tracing::warn!(action = unknown, "unknown keymap action — ignoring");
            None
        }
    }
}
