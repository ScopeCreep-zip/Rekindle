//! Rekindle's items for the engine menus a skin opens (plan SK2-A, lead
//! decision 36). Menu *items* were engine resources, not pack content
//! (`v3/strings.xml` `XfireMenu` 228, `ToolsMenu` 229, `HelpMenu` 231,
//! `GCActionsMenu` 436): a pack names which [`EngineMenu`] a tile opens,
//! and [`MenuRegistry`] holds Rekindle's items for it. Re-exported from
//! [`super::skin_action`].

use serde::{Deserialize, Serialize};

use super::skin_action::{EngineMenu, EngineView, PackUrl, SkinAction, SkinSettingsTab};
use crate::presence::SessionStatus;

/// One item of an engine menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuItem {
    pub label: String,
    pub run: SkinAction,
    /// A separator above the item.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub apart: bool,
    /// A destructive item, drawn as such.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub danger: bool,
}

/// One engine menu's items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineMenuItems {
    pub menu: EngineMenu,
    pub items: Vec<MenuItem>,
}

/// Rekindle's items for each engine menu a pack opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuRegistry {
    pub menus: Vec<EngineMenuItems>,
}

fn item(label: &str, run: SkinAction) -> MenuItem {
    MenuItem {
        label: label.to_owned(),
        run,
        apart: false,
        danger: false,
    }
}

fn apart(mut i: MenuItem) -> MenuItem {
    i.apart = true;
    i
}

impl MenuRegistry {
    /// The engine menus with no Rekindle items yet: the chat window's
    /// (`CwActions`, `CwHelp`) and the group chat window's (`GcSystem`,
    /// `GcActions`, `GcHelp`) menus. They are planned feature gaps (design
    /// §2.2: built from the chat windows' existing actions); the pack's
    /// buttons for them stay.
    pub const FEATURE_GAPS: [EngineMenu; 5] = [
        EngineMenu::CwActions,
        EngineMenu::CwHelp,
        EngineMenu::GcSystem,
        EngineMenu::GcActions,
        EngineMenu::GcHelp,
    ];

    /// Rekindle's menus: File = the buddy list's first menu
    /// (`PackMenus.tsx` `first` + `MenuBar.tsx` Settings tabs and Quit),
    /// Tool = `tools` + `MenuBar.tsx` "Calls", Help = `help`, Status = the
    /// four presence choices, Sys = the window's system menu.
    #[must_use]
    pub fn rekindle() -> Self {
        use SkinAction as A;
        let file = vec![
            item("Add Friend…", A::AddFriend),
            item("New Chat…", A::NewChat),
            item("New Group…", A::NewGroup),
            apart(item("Create Community…", A::CreateCommunity)),
            item("Join Community…", A::JoinCommunity),
            apart(item("Profile", A::ShowSettings(SkinSettingsTab::Profile))),
            item("Privacy", A::ShowSettings(SkinSettingsTab::Privacy)),
            item("About", A::ShowSettings(SkinSettingsTab::About)),
            apart(item("Log Off", A::LogOff)),
            MenuItem {
                danger: true,
                ..item("Exit", A::Quit)
            },
        ];
        let tool = vec![
            item("Options", A::ShowSettings(SkinSettingsTab::Application)),
            item(
                "Notifications",
                A::ShowSettings(SkinSettingsTab::Notifications),
            ),
            item("Audio", A::ShowSettings(SkinSettingsTab::Audio)),
            item("Skins", A::ShowSettings(SkinSettingsTab::Appearance)),
            apart(item("Start Group Call…", A::HostGroupVoiceChat)),
            item("Keyboard Shortcuts", A::ShowShortcuts),
        ];
        let help = vec![
            item("Documentation", A::ShowView(EngineView::Help)),
            item("Report Bug", A::ReportBug),
        ];
        let status = vec![
            item("Online", A::SetStatus(SessionStatus::Online)),
            item("Away", A::SetStatus(SessionStatus::Away)),
            item("Busy", A::SetStatus(SessionStatus::Busy)),
            item("Appear Offline", A::SetStatus(SessionStatus::Invisible)),
        ];
        let sys = vec![
            item("Minimize", A::Minimize),
            item("Maximize", A::Maximize),
            apart(item("Close", A::Close)),
        ];
        let mut menus: Vec<EngineMenuItems> = [
            (EngineMenu::File, file),
            (EngineMenu::Tool, tool),
            (EngineMenu::Help, help),
            (EngineMenu::Status, status),
            (EngineMenu::Sys, sys),
        ]
        .into_iter()
        .map(|(menu, items)| EngineMenuItems { menu, items })
        .collect();
        menus.extend(Self::FEATURE_GAPS.map(|menu| EngineMenuItems {
            menu,
            items: Vec::new(),
        }));
        Self { menus }
    }

    /// Rekindle's menus for the active pack named `pack` (plan SK10,
    /// decision 49): [`Self::rekindle`], with one more Help item,
    /// "<pack>'s website", running the pack's own link `site` (its logo's
    /// `LaunchUrl`, which is pointer-only once the logo is no button around
    /// the window menu), so a keyboard user reaches it too. No item when the
    /// pack declares no link.
    #[must_use]
    pub fn for_pack(pack: &str, site: Option<&PackUrl>) -> Self {
        let mut reg = Self::rekindle();
        if let Some(site) = site {
            if let Some(help) = reg.menus.iter_mut().find(|m| m.menu == EngineMenu::Help) {
                help.items.push(apart(item(
                    &format!("{pack}'s website"),
                    SkinAction::LaunchUrl(site.clone()),
                )));
            }
        }
        reg
    }

    /// `menu`'s items (empty for a [`Self::FEATURE_GAPS`] menu).
    #[must_use]
    pub fn items(&self, menu: EngineMenu) -> &[MenuItem] {
        self.menus
            .iter()
            .find(|m| m.menu == menu)
            .map_or(&[][..], |m| m.items.as_slice())
    }
}

#[cfg(test)]
#[path = "skin_menu_tests.rs"]
mod tests;
