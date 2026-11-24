//! The chat window's name → role table (plan SK17-2): the title strip, its
//! window buttons and menu row, and the slots the shell mounts the
//! conversation into, read from each pack's own chat-window vocabulary
//! (Onyx's and Space Cowboy's `chatwindow.xml`, xSLIMFIRE's
//! `xSLIMFIREchatwindow.xml`, Symbiosis's `ChatWindow.xml`), never the
//! buddy list's. A name this table lacks goes to the buddy list's table
//! ([`super::roles::role`]: the frame art, the grips, the drag strip).

use rekindle_types::dto::skin_action::{EngineMenu, NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{NodeRole, SlotKey, WindowButtonKind};

use super::roles::{Facts, Role};

/// A pack name's table key: lower case, its trailing digits dropped
/// (`MenuDiv0`…`MenuDiv3`, `Separator1`).
fn key(name: &str) -> String {
    name.trim_end_matches(|c: char| c.is_ascii_digit())
        .to_ascii_lowercase()
}

fn acts(shown: NodeRole, on: SkinEvent, run: SkinAction) -> Role {
    Role::Acts(shown, Some(NodeTrigger { on, run }))
}

/// A window button: the pack's art states, run by a click.
fn button(kind: WindowButtonKind) -> Role {
    let run = match kind {
        WindowButtonKind::Minimize => SkinAction::Minimize,
        WindowButtonKind::Maximize => SkinAction::Maximize,
        WindowButtonKind::Close => SkinAction::Close,
    };
    acts(NodeRole::WindowButton(kind), SkinEvent::Click, run)
}

/// A menu title popping the chat window's `menu` on the button press.
fn menu(m: EngineMenu) -> Role {
    acts(
        NodeRole::MenuTitle,
        SkinEvent::LeftDown,
        SkinAction::Menu(m),
    )
}

fn by_key(k: &str) -> Option<Role> {
    use NodeRole as R;
    use WindowButtonKind as W;
    let shows = |r| Some(Role::Shows(r));
    match k {
        "chatwindow" | "chatmainwindow" | "topbuttonbar" | "chatusercontainer"
        | "chatbaseboard" | "contentcontainer" | "textcontainer" | "chatbuttonbar" => {
            shows(R::Group)
        }
        "chattitlebar" | "maintitlebar" | "infobar" => shows(R::TitleBar),
        "logo" | "xfirelogo" | "xfirelogonolink" => shows(R::Logo),
        "chattitle" | "chatuser" | "usernamefield" => shows(R::ChatTitle),
        "minimize" | "minimizebox" | "buttonminimize" => Some(button(W::Minimize)),
        "maximize" | "maximizebox" | "buttonmaximize" => Some(button(W::Maximize)),
        "chatclose" | "closechatbox" | "closebox" | "buttonclose" => Some(button(W::Close)),
        "menubar" | "chatmenubar" | "menubackground" => shows(R::MenuBar),
        // The first menu is Rekindle's where the pack's reads "Xfire"
        // (Onyx's and Space Cowboy's `ChatXfireMenuText`, Symbiosis's
        // `XfireMenu`): the brand rule renames only that word. xSLIMFIRE's
        // `SystemMenuText` reads "System" (its `<Text>GCSystemMenu</Text>`)
        // and its plate runs the engine's `MenuSystem`
        // (`xSLIMFIREcomponents.xml:1076-1099`): the window's system menu,
        // its word kept. Then the chat window's own menus.
        "chatxfiremenutext" | "xfiremenu" => Some(menu(EngineMenu::File)),
        "systemmenutext" => Some(menu(EngineMenu::Sys)),
        "chattoolsmenutext" => Some(menu(EngineMenu::Tool)),
        "actionsmenutext" | "actionsmenu" => Some(menu(EngineMenu::CwActions)),
        "chathelpmenutext" | "helpmenutext" | "helpmenu" => Some(menu(EngineMenu::CwHelp)),
        "menudiv" | "divider" | "horizontalseparator" | "buttonbarverticalseparator" => {
            shows(R::Decor)
        }
        "tabbedtalkingstate" => shows(R::TalkingState),
        "chatarea" => shows(R::Slot(SlotKey::ChatArea)),
        "chatedit" => shows(R::Slot(SlotKey::ChatEdit)),
        "typingmessage" | "typingmessagebg" | "chattypingmessage" => {
            shows(R::Slot(SlotKey::TypingNotice))
        }
        // Onyx's and Space Cowboy's `ChatImButton` runs `SendMessage`.
        "sendbutton" | "chatbuttonsend" | "sendchatbutton" | "chatimbutton" => {
            shows(R::Slot(SlotKey::ChatSend))
        }
        "chatinfoviewbutton" | "infoviewbutton" | "chatexpandbutton" | "expandbutton" => {
            shows(R::Slot(SlotKey::InfoviewButton))
        }
        _ => None,
    }
}

/// The role of a chat-window tile named `name` with component
/// `component` under a parent of role `parent`; `None` sends it to the
/// buddy list's table. A menu title's, a chat title's, a talking lamp's
/// and a slot's children are its parts (the menu's hover plate
/// `FileMenu`, xSLIMFIRE's `UserNameText`, Symbiosis's
/// `ChatButtonTalking`); a slot fills its box whole.
#[must_use]
pub fn role(
    name: &str,
    component: Option<&str>,
    parent: Option<&Role>,
    f: &Facts<'_>,
) -> Option<Role> {
    use NodeRole as R;
    if parent.is_some_and(|p| {
        matches!(
            p.shown(),
            R::MenuTitle | R::ChatTitle | R::TalkingState | R::Slot(_) | R::WindowButton(_)
        )
    }) {
        return Some(Role::Shows(R::Part));
    }
    let found = component
        .map(key)
        .and_then(|c| by_key(&c))
        .or_else(|| by_key(&key(name)));
    found.or_else(|| {
        // A pack's own container: a group when it holds tiles (Space
        // Cowboy's `ListViewBG2` around `ChatEdit`), its art else.
        (!f.children.is_empty() && matches!(key(name).as_str(), "listviewbg" | "chateditbg"))
            .then_some(Role::Shows(R::Group))
    })
}
