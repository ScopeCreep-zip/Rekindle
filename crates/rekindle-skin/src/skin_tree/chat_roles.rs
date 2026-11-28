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
        | "chatbaseboard" | "contentcontainer" | "textcontainer" | "chatbuttonbar"
        // The INI packs' containers (plan SK17-3): Combat's `[ContentArea]`
        // around `ChatArea`/`ChatEdit` (`CombatChatSkin.ini:288`), Alloy's
        // and Simple X's `[BaseBoard]` (`Chatskin.ini:459`, `:414`).
        | "contentarea" | "baseboard" => {
            shows(R::Group)
        }
        "chattitlebar" | "maintitlebar" | "infobar" => shows(R::TitleBar),
        // Alloy's and Simple X's `[ChatLogo]` (`Chatskin.ini:167`, `:150`).
        "logo" | "xfirelogo" | "xfirelogonolink" | "chatlogo" => shows(R::Logo),
        // StealthX's `UserNameText` (`chatwindow.xml:11`) is a text whose
        // string is the engine's `UserNameText` (`components.xml:899-901`):
        // the conversation's user name, the chat title.
        // Combat's chat `[Title]` reads `Xfire - %displayuser%`
        // (`CombatChatSkin.ini:243-249`): the conversation's title, served
        // with no words of the pack's.
        "chattitle" | "chatuser" | "usernamefield" | "usernametext" | "title" => {
            shows(R::ChatTitle)
        }
        "minimize" | "minimizebox" | "buttonminimize" => Some(button(W::Minimize)),
        "maximize" | "maximizebox" | "buttonmaximize" => Some(button(W::Maximize)),
        // Combat's `[Close]` `Class=PushButton Action.click=AppClose`
        // (`CombatChatSkin.ini:279-286`), in its `[MainTitleBar]` (`:236`).
        "chatclose" | "closechatbox" | "closebox" | "buttonclose" | "close" => {
            Some(button(W::Close))
        }
        // Combat's `[GCMenuBar]` (`CombatChatSkin.ini:454`).
        "menubar" | "chatmenubar" | "menubackground" | "gcmenubar" => shows(R::MenuBar),
        // The first menu is Rekindle's where the pack's reads "Xfire"
        // (Onyx's and Space Cowboy's `ChatXfireMenuText`, Symbiosis's
        // `XfireMenu`): the brand rule renames only that word. xSLIMFIRE's
        // `SystemMenuText` reads "System" (its `<Text>GCSystemMenu</Text>`)
        // and its plate runs the engine's `MenuSystem`
        // (`xSLIMFIREcomponents.xml:1076-1099`): the window's system menu,
        // its word kept. Then the chat window's own menus.
        "chatxfiremenutext" | "xfiremenu" => Some(menu(EngineMenu::File)),
        // Combat's `GC…MenuText` titles (`CombatChatSkin.ini:478`, `:508`,
        // `:538`) are the INI packs' `…MenuText` ones.
        "systemmenutext" | "gcsystemmenutext" => Some(menu(EngineMenu::Sys)),
        "chattoolsmenutext" => Some(menu(EngineMenu::Tool)),
        "actionsmenutext" | "actionsmenu" | "gcactionsmenutext" => {
            Some(menu(EngineMenu::CwActions))
        }
        // StealthX's chat menu popup names its help title `HelpMenuCText`
        // over a `HelpMenuC` plate running `MenuCWHelp`
        // (`components.xml:884-890`, `popups.xml:107`).
        "chathelpmenutext" | "helpmenutext" | "helpmenu" | "helpmenuctext" | "gchelpmenutext" => {
            Some(menu(EngineMenu::CwHelp))
        }
        // Simple X's `IVButtonHolder` (`Chatskin.ini:99-104,470-474`): a
        // plain `Class=Tile` drawing `info-button-holder.gif`, the plate
        // the info-view button sits on; art only, no action.
        "menudiv" | "divider" | "horizontalseparator" | "buttonbarverticalseparator"
        | "ivbuttonholder" => {
            shows(R::Decor)
        }
        "tabbedtalkingstate" => shows(R::TalkingState),
        "chatarea" => shows(R::Slot(SlotKey::ChatArea)),
        "chatedit" => shows(R::Slot(SlotKey::ChatEdit)),
        // Doom3 UAC-PDA's `[TYPING]` `Class=Text` shows the engine's
        // `TypingMessage` string on `ActiveUserTyping` (`ChatSkin.ini:227-235`).
        "typingmessage" | "typingmessagebg" | "chattypingmessage" | "typing" => {
            shows(R::Slot(SlotKey::TypingNotice))
        }
        // Onyx's and Space Cowboy's `ChatImButton` runs `SendMessage`.
        // Combat's `[ImButton]` (`CombatChatSkin.ini:343`, plan E7.7.7 S3)
        // and XGlow's `[Send]` (`ChatSkin.ini:290`) likewise.
        "sendbutton" | "chatbuttonsend" | "sendchatbutton" | "chatimbutton" | "imbutton"
        | "send" => {
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
        // XGlow's `[ViewInfoButton]` (`ChatSkin.ini:248`).
        "chatinfoviewbutton" | "infoviewbutton" | "chatexpandbutton" | "expandbutton"
        | "viewinfobutton" => {
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
    let shown = parent.map(Role::shown);
    if shown.is_some_and(|p| {
        matches!(
            p,
            R::MenuTitle
                | R::ChatTitle
                | R::TalkingState
                | R::Slot(_)
                | R::WindowButton(_)
                | R::Part
        )
    }) {
        // The typing line a pack lays in its title field is the typing
        // slot, never a part of the title: XGlow's `[UserNameField]`
        // holds `UserNameText` and `TypingMessage` (`ChatSkin.ini:154-166`),
        // Alloy's `[UserNameBG]` in its `[UserNameField]` likewise
        // (`Chatskin.ini:181-210`).
        let typing = by_key(&key(name)) == Some(Role::Shows(R::Slot(SlotKey::TypingNotice)));
        if typing && matches!(shown, Some(R::ChatTitle | R::Part)) {
            return Some(Role::Shows(R::Slot(SlotKey::TypingNotice)));
        }
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
