//! The chat window's name → role table (plan SK17-2): the title strip, its
//! window buttons and menu row, and the slots the shell mounts the
//! conversation into, read from each pack's own chat-window vocabulary
//! (Onyx's and Space Cowboy's `chatwindow.xml`, xSLIMFIRE's
//! `xSLIMFIREchatwindow.xml`, Symbiosis's `ChatWindow.xml`), never the
//! buddy list's. A name this table lacks goes to the buddy list's table
//! ([`super::roles::role`]: the frame art, the grips, the drag strip).

use rekindle_types::dto::skin_action::{EngineMenu, NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{NodeRole, SlotKey, WindowButtonKind};

use super::roles::{key, Facts, Role};

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
        // StealthX's `UserNameText` (`chatwindow.xml:11`) is a text whose
        // string is the engine's `UserNameText` (`components.xml:899-901`):
        // the conversation's user name, the chat title.
        "chattitle" | "chatuser" | "usernamefield" | "usernametext" => shows(R::ChatTitle),
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
        // StealthX's chat menu popup names its help title `HelpMenuCText`
        // over a `HelpMenuC` plate running `MenuCWHelp`
        // (`components.xml:884-890`, `popups.xml:107`).
        "chathelpmenutext" | "helpmenutext" | "helpmenu" | "helpmenuctext" => {
            Some(menu(EngineMenu::CwHelp))
        }
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
        // StealthX's `ChatMenuButton` (`chatwindow.xml:8`) is the chat
        // window's one menu: a `SkinLauncher` whose `LaunchSkin` opens its
        // `ChatMenuPopup` (`components.xml:838-852`, `popups.xml:107`), the
        // buddy list's `MenuButton` → `MainMenuPopup` shape: a button run
        // by the pack's own trigger. StealthX's chat menu popup's
        // `GameButton1` (`popups.xml:107`) is a `PushButton` whose pack
        // trigger runs `JoinFriend` (`components.xml:184-188`): a button
        // run by that trigger too.
        "chatmenubutton" | "groupchatmenubutton" | "gamebutton" => {
            Some(Role::Acts(R::Button, None))
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
