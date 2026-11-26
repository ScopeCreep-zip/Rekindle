//! The pack-name → role table (plan SK2, design §1 and §5 "Import"):
//! keyed by the tile's `Component=` (an INI `[Section]`), else its
//! `Name=`, ASCII-case-insensitively with trailing digits dropped
//! (Classic's `GameButton1`/`GameButton2`, Combat's `Knob01`).

use rekindle_types::dto::skin_action::{EngineMenu, NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Edge, LampKey, NodeRole, SkinNode, ViewKey};

/// What the name table makes of a tile (plan SK2 decision 36, design
/// §2.4): a role that only shows, or a control that acts: its shown role
/// (`Button`, `MenuTitle`) and the trigger its name implies,
/// which the importer adds when the pack wires no pointer trigger of its
/// own (a pack's own triggers win). `None` for a control whose action
/// needs the pack's data (a pack link's URL, a popup's name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Shows(NodeRole),
    Acts(NodeRole, Option<NodeTrigger>),
}

impl Role {
    /// The role the node shows.
    #[must_use]
    pub const fn shown(&self) -> NodeRole {
        match self {
            Self::Shows(r) | Self::Acts(r, _) => *r,
        }
    }

    /// Whether the tile takes input: a control, or an input role.
    #[must_use]
    pub const fn interactive(&self) -> bool {
        match self {
            Self::Shows(r) => r.input(),
            Self::Acts(..) => true,
        }
    }
}

/// `triggers` (the pack's) then the trigger the tile's name implies
/// (design §2.4), unless the pack wires a pointer trigger of its own on the
/// tile or on one of its parts (StealthX's event-transparent
/// `XfireMenuText` label holds the `FileMenu` part that runs `MenuFile`,
/// `popups.xml:5-6`): a control the name table knows keeps its behaviour
/// as a trigger, and a pack's own wiring wins.
#[must_use]
pub fn with_implied(
    mut triggers: Vec<NodeTrigger>,
    role: &Role,
    children: &[SkinNode],
) -> Vec<NodeTrigger> {
    let wired = |ts: &[NodeTrigger]| ts.iter().any(NodeTrigger::presses);
    let parts_wired = children
        .iter()
        .any(|c| c.role == NodeRole::Part && wired(&c.triggers));
    if let Role::Acts(_, Some(implied)) = role {
        if !wired(&triggers) && !parts_wired {
            triggers.push(implied.clone());
        }
    }
    triggers
}

/// A control `shown` as, run by `on` → `run`.
fn acts(shown: NodeRole, on: SkinEvent, run: SkinAction) -> Role {
    Role::Acts(shown, Some(NodeTrigger { on, run }))
}

/// A button run by a click.
fn click(run: SkinAction) -> Role {
    acts(NodeRole::Button, SkinEvent::Click, run)
}

/// A menu title popping `menu` on the button press (Xfire's
/// `OnLeftButtonDown` → `Menu<Name>`).
fn menu(m: EngineMenu) -> Role {
    acts(
        NodeRole::MenuTitle,
        SkinEvent::LeftDown,
        SkinAction::Menu(m),
    )
}

/// What a tile is, beyond its names: whether it has art, its class and
/// its children's names.
pub struct Facts<'a> {
    pub has_art: bool,
    pub class: Option<&'a str>,
    pub children: &'a [String],
}

/// A pack name's table key: lower case, its trailing digits dropped
/// (`MenuDiv0`…`MenuDiv3`, `Separator1`); the main and chat tables share it.
pub(super) fn key(name: &str) -> String {
    name.trim_end_matches(|c: char| c.is_ascii_digit())
        .to_ascii_lowercase()
}

fn view(name: &str) -> Option<ViewKey> {
    let n = name.to_ascii_lowercase();
    [
        ("chatroom", ViewKey::ChatRooms),
        ("picture", ViewKey::Pictures),
        ("video", ViewKey::Videos),
        ("notification", ViewKey::Notifications),
        ("server", ViewKey::Servers),
        ("download", ViewKey::Files),
        ("file", ViewKey::Files),
        ("friend", ViewKey::Friends),
        ("buddy", ViewKey::Friends),
        ("normal", ViewKey::Friends),
    ]
    .into_iter()
    .find_map(|(k, v)| n.contains(k).then_some(v))
}

/// Combat's knob stops in rotation order (`combat.ini` `[Knob01]`-`[Knob04]`,
/// each a sticky `PushButton` of `GroupId=2`): Friends, Servers, Files, and
/// the return sweep through Servers to Friends. `Knob03` turns to `Knob04`
/// with the Servers view (`GroupMessage(3)`, `GroupMessage(1,1)`), `Knob04`
/// draws the Servers pointer (`c_knob_02.gif`, `Knob02`'s art) and turns on
/// to Friends (`GroupMessage(0)`, `GroupMessage(0,1)`).
const KNOBS: [ViewKey; 4] = [
    ViewKey::Friends,
    ViewKey::Servers,
    ViewKey::Files,
    ViewKey::Servers,
];

fn button(k: &str) -> Option<SkinAction> {
    use SkinAction as A;
    let rows: [(&[&str], SkinAction); 25] = [
        // StealthX's rail (`popups.xml:69-79`), ahead of "chat" and "join".
        // Classic's info popup spells them `InfoHostChatButton` … (`xfirepopups.xml:8-10`).
        (&["hostchat", "infohostchat"], A::HostVoiceChat),
        (&["joinchat", "infojoinchat"], A::VoiceChat),
        (&["leavechat", "infoleavechat"], A::LeaveVoiceChat),
        (&["addserver", "addfav", "favorite"], A::AddServer),
        (&["startdownload"], A::StartDownload),
        (&["stopdownload"], A::StopDownload),
        // Combat's `ChatRefreshStopTab` on the Files tab (lead decision 1).
        (&["stop"], A::StopDownload),
        // Symbiosis's `ButtonViewPictures`: open the selected shared file.
        (&["viewpicture", "viewvideo"], A::OpenDownload),
        (&["executedownload", "openfile"], A::OpenDownload),
        (&["downloadinfo"], A::DownloadInfo),
        // StealthX's rail `DownloadButton` (`Tooltip="StartDownloadButtonTip"`)
        // and `ExecuteButton` (`ExecuteDownloadButtonTip`).
        (&["download"], A::StartDownload),
        (&["execute"], A::OpenDownload),
        // Symbiosis's Pictures/Videos rows: a shared file's info, and
        // sharing one (the attachment upload) (lead decision 1).
        (&["infopicture", "infovideo"], A::DownloadInfo),
        (&["upload"], A::UploadSelected),
        (&["bigbrowsefile", "browse"], A::UploadSelected),
        (&["find", "search"], A::SearchFor),
        (&["roomenter", "enterroom", "enter"], A::EnterRoom),
        (&["roomcreate", "create"], A::CreateRoom),
        (&["delete"], A::DeleteSelected),
        (&["refresh"], A::RefreshServer),
        (&["add"], A::AddFriend),
        (&["chat", "im"], A::ChatFriend),
        (&["game", "join", "play"], A::JoinFriend),
        (&["info", "profile"], A::ProfileFriend),
        (&["fav"], A::AddServer),
    ];
    let k = k
        .trim_start_matches("button")
        .trim_start_matches("tab")
        .trim_end_matches("button");
    rows.into_iter()
        .find_map(|(keys, a)| keys.iter().any(|p| k.starts_with(p)).then_some(a))
}

/// Every child is a chat-state button (host, join, leave).
fn chat_states(children: &[String]) -> bool {
    !children.is_empty()
        && children.iter().all(|c| {
            matches!(
                button(&key(c)),
                Some(
                    SkinAction::HostVoiceChat | SkinAction::VoiceChat | SkinAction::LeaveVoiceChat
                )
            )
        })
}

/// A tab outside a tab strip: Classic's `TabNormalText`/`TabNormalButton`
/// pairs in Onyx's menu bar and Alloy's tab bar, Symbiosis's
/// `ButtonNotificationsTab` beside the name, XGlow's `[Friends]`.
fn tab(k: &str) -> Option<()> {
    let paired = (k.starts_with("tab") && (k.ends_with("text") || k.ends_with("button")))
        || (k.starts_with("button") && k.ends_with("tab"));
    (paired || matches!(k, "friends" | "servers" | "downloads" | "files")).then_some(())
}

/// The edge part of a grip's key (`MainResizeGripperBL`, `ResizeUp`,
/// `GripperTopLeft`, `BRGripper`), `None` for a key that names no grip.
fn grip(k: &str) -> Option<&str> {
    k.strip_prefix("mainresizegripper")
        .or_else(|| k.strip_prefix("popupresizegripper"))
        // The info popup's grip: Classic's `InfoInfoResizeRight` on its
        // `InfoResizeRight` tile (`xfirepopups.xml`, `ResizeR`,
        // `xfirecomponents.xml:1862-1870`); its `InfoGripperTR`/`BR` are
        // corner art ([`named_shown`]).
        .or_else(|| k.strip_prefix("infoinforesize"))
        .or_else(|| k.strip_prefix("inforesize"))
        .or_else(|| k.strip_prefix("resizegripper"))
        .or_else(|| k.strip_prefix("resize"))
        .or_else(|| k.strip_prefix("gripper"))
        .or_else(|| k.strip_suffix("gripper"))
}

/// The closed table of edge spellings the packs use (Symbiosis's
/// `MainResizeGripperBL`/`TL`/`TR`/`L`/`R` and popup `PopupResizeGripperR`
/// (Symbiosis `Popups.xml:16`), Classic's `ResizeUp`, XGlow's `[ResizeTop]`): an edge outside it is an import error
/// ([`check_grip`]), never a guessed corner.
fn edge(rest: &str) -> Option<Edge> {
    Some(match rest {
        "up" | "top" | "t" => Edge::Top,
        "down" | "bottom" | "b" => Edge::Bottom,
        "left" | "l" => Edge::Left,
        "right" | "r" => Edge::Right,
        "topleft" | "tl" | "ul" => Edge::TopLeft,
        "topright" | "tr" | "ur" => Edge::TopRight,
        "bottomleft" | "bl" => Edge::BottomLeft,
        "bottomright" | "br" => Edge::BottomRight,
        _ => return None,
    })
}

/// Refuses a tile the pack wires no pressing trigger on whose name or
/// component is a grip with an edge outside [`edge`]'s table (plan SK2-A4:
/// the old `_ => BottomRight` default misread five Symbiosis grips). A
/// grip the pack wires itself (XGlow's `[Gripper]`, Combat's
/// `[ResizeUlbr]`: `Action.lbtndown=ResizeBR`) needs no edge from its
/// name.
///
/// # Errors
/// The refusal, naming the tile and the grip's key.
pub fn check_grip(
    name: &str,
    component: Option<&str>,
    triggers: &[NodeTrigger],
) -> Result<(), String> {
    if triggers.iter().any(NodeTrigger::presses) {
        return Ok(());
    }
    [Some(name), component]
        .into_iter()
        .flatten()
        .map(key)
        .find(|k| grip(k).is_some_and(|rest| edge(rest).is_none()))
        .map_or(Ok(()), |k| Err(format!("{name}: grip {k}: unknown edge")))
}

/// A grip: the resize its name's edge implies, or, for a name without a
/// known edge, a button whose action is the pack's own trigger
/// ([`check_grip`] refuses one the pack leaves unwired).
fn resize(k: &str) -> Option<Role> {
    Some(
        edge(grip(k)?).map_or(Role::Acts(NodeRole::Button, None), |e| {
            acts(NodeRole::Button, SkinEvent::LeftDown, SkinAction::Resize(e))
        }),
    )
}

fn named(k: &str, f: &Facts<'_>) -> Option<Role> {
    use NodeRole as R;
    use SkinAction as A;
    named_shown(k, f).map(Role::Shows).or_else(|| match k {
        "minimizebox" | "minimize" | "buttonminimize" | "wndmin" | "minimizebutton" => {
            Some(click(A::Minimize))
        }
        "maximizebox" | "maximize" | "buttonmaximize" | "wndmax" => Some(click(A::Maximize)),
        "closebox" | "close" | "buttonclose" | "wndclose" => Some(click(A::Close)),
        "xfiremenutext" | "xfiremenu" | "filemenutext" | "filemenu" => Some(menu(EngineMenu::File)),
        "toolsmenutext" | "toolsmenu" | "toolmenu" => Some(menu(EngineMenu::Tool)),
        "helpmenutext" | "helpmenu" => Some(menu(EngineMenu::Help)),
        // The InfoView toggle and StealthX's rail toggle launch the pack's
        // popup (`LaunchSkin` + `<Popup>`); a pack link opens the pack's
        // URL (`AppLaunchUrl`): both from the pack's own triggers.
        k if info_toggle(k) => Some(Role::Acts(R::Button, None)),
        // XGlow's popup `[Voice]` runs the pack's `Action.click=AppVoiceChat`
        // (`XGlowInfoSkin.ini:168-181`).
        // Combat's info popup `[JoinChatButton]` `Class=PushButton` runs the
        // pack's own `Action.click=AppVoiceChat` (`CombatInfoSkin.ini:136-146`,
        // `CombatGroupChatInfoSkin.ini:150-160`), outside any button row.
        "menubutton" | "tclink" | "voice" | "joinchatbutton" => Some(Role::Acts(R::Button, None)),
        "systemmenu" => Some(acts(
            R::Button,
            SkinEvent::LeftDown,
            A::ContextMenu(EngineMenu::Sys),
        )),
        // The info popup's join button: the selected friend's game
        // (Classic's `InfoJoinGameButton`, Space Cowboy's and Onyx's
        // `JoinButton`, Total Chaos's `InfoJoinButton`, XGlow's `[Join]`
        // `Class=PushButton` with `Graphics.Trigger=AppCanJoin`,
        // `XGlowInfoSkin.ini:185-189`; its main window's `[Join]` is a
        // `Class=TabControl`, `Xglow.ini:616-618`, laid as a tab control above).
        "joinbutton" | "infojoingamebutton" | "infojoinbutton" | "joingamebutton" | "join" => {
            Some(click(A::JoinFriend))
        }
        "adddownloadtab" => Some(click(A::AddFriend)),
        "chatrefreshstoptab" => Some(click(A::ChatFriend)),
        "joinplaytab" => Some(click(A::JoinFriend)),
        "profilebrowsetab" => Some(click(A::ProfileFriend)),
        _ => None,
    })
}

/// Whether `k` names the InfoView toggle (`InfoViewButton`,
/// `ExpandButton`, Combat's tab-control `InfoViewSwitch`). Alloy's
/// `IVButtonHolder` is not one: `Class=Tile` art with no trigger, the
/// plate behind the button (`Chatskin.ini:523-527`), so pack decor.
fn info_toggle(k: &str) -> bool {
    matches!(
        k,
        "infoviewbutton"
            | "expandbutton"
            | "infoviewswitch"
            | "viewinfo"
            | "buttonmaininfoout"
            // Inside the popup, the toggle that closes it: Symbiosis's
            // `ButtonMainInfoIn` (`Popups.xml:26`), Total Chaos's and Onyx's
            // `CloseInfoView` (`Class="SkinLauncher"`, TC `components.xml:825`).
            | "buttonmaininfoin"
            | "closeinfoview"
    )
}

fn named_shown(k: &str, f: &Facts<'_>) -> Option<NodeRole> {
    use NodeRole as R;
    match k {
        "maintitlebar" | "windowdragger" | "combatdragbar" => Some(R::Drag),
        "xfirelogo" | "xfirelogonolink" | "logo" | "logox" | "wndxfire" => Some(R::Logo),
        // A `Title` is the user's name only as text (Combat's `[Title]`
        // `Class=Text Text.text=%displayuser%`, `combat.ini:541-544`; Onyx's
        // and Space Cowboy's `Class="Text"`); Total Chaos's `Title` is the
        // "T.C" plaque, `Class="Tile"` art (`components.xml:451-460`), and its
        // original shows no name: classless art, so decor.
        "title" | "chattitle"
            if !f
                .class
                .is_some_and(|c| c.trim().eq_ignore_ascii_case("Text")) =>
        {
            None
        }
        "userfield" | "usernamefield" | "title" | "chattitle" | "usernamecontainer"
        | "username" => Some(R::NameField),
        // StealthX's rail ground holds the menu bar: it is not the bar.
        "menubackground" if f.children.iter().any(|c| key(c) == "menubar") => Some(R::Decor),
        "menubar" | "menubackground" | "menubg" => Some(R::MenuBar),
        "statusedit" | "status" => Some(R::StatusText),
        "downarrow" | "statusbtn" | "buttonstatuschanger" => Some(R::StatusList),
        // The ad's container and its view: the inner one is its part
        // (Symbiosis's `AdViewContainer` > `AdView`, Total Chaos's
        // `AdBar` > `AdArea`).
        "adarea" | "adbar" | "adview" | "adviewcontainer" | "adareabg" | "adbg" => Some(R::AdArea),
        // Window art (design §1 rows 2, 5, 18, 41): the frame and its decor.
        "topleft" | "top" | "topright" | "leftedge" | "rightedge" | "bottomleft" | "bottom"
        | "bottomright" | "innerbevel" | "topbevel" | "middlebevel" | "bottombevel"
        | "leftouteredge" | "rightouteredge" | "topouteredge" | "bottomouteredge"
        | "mainrightouteredge" | "mainleftouteredge" | "middle" | "mainbg" | "outerframe"
        | "left" | "right" | "maintoprow" | "mainleftside" | "mainbackground" | "mainrightside"
        | "mainbottomrow" | "mainbackgroundcol" | "leftdetail" | "rightdetail" | "vent"
        | "divider" | "partnergraphic" | "partnerframe" | "agear" | "igear" | "sclogo" | "thex"
        | "thexcover"
        // Combat's info popup backdrop: `[BackDrop]` `Class=Tile`
        // `Graphics.middle=middle.gif` behind the pane at `Z=-2`
        // (`CombatInfoSkin.ini:23-30,95-98`; `CombatGroupChatInfoSkin.ini:92-94`
        // `c_background.gif`): window art, its bitmap absent from the pack
        // (spec A8: an undefined bitmap lays nothing).
        | "backdrop" => Some(R::Decor),
        // The info popup's corner art, not a grip (SK2-P1f): Classic's
        // `InfoInfoGripperTR`/`BR` (`xfirecomponents.xml:1962-1977`),
        // xSLIMFIRE's (`xSLIMFIREcomponents.xml:1450-1468`) and Combat's
        // `[InfoGripperTR]`/`[InfoGripperBR]` (`CombatInfoSkin.ini:111-121`)
        // are `Hilight` art with no trigger and no `Cursor`, where every
        // grip Xfire resizes by wires its own (`InfoInfoResizeRight`
        // `ResizeR`, `xfirecomponents.xml:1862-1870`; Symbiosis's
        // `MainResizeGripperTL` `ResizeUL`, `Components/MainWindow.xml:62-66`),
        // and `Xfire.exe` names no grip tile outside its embedded skin.
        // Combat's `INFOVIEWGRIPPERTOPRIGHT` is defined neither by the pack
        // nor by that embedded skin (`shadowthemes.xml`), so it lays nothing
        // (spec A8).
        "infogrippertr" | "infogripperbr" | "infoinfogrippertr" | "infoinfogripperbr" => {
            Some(R::Decor)
        }
        "tabheader" | "tabs" | "tabbuttonbar" => Some(R::TabStrip),
        "tabcontrol" if f.children.iter().any(|c| key(c) == "knob") => Some(R::TabStrip),
        // Symbiosis's one list for every tab (lead decision 9): the views,
        // split into one view per tab by `passes::tab_views`.
        "xfireview"
            if f.class
                .is_some_and(|c| c.trim().eq_ignore_ascii_case("XfireView")) =>
        {
            Some(R::Views)
        }
        "buddyview" | "xfireview" => Some(R::View(ViewKey::Friends)),
        "serverview" => Some(R::View(ViewKey::Servers)),
        "downloadview" => Some(R::View(ViewKey::Files)),
        "buttonrow"
        | "friendsbuttonbar"
        | "serversbuttonbar"
        | "filesbuttonbar"
        | "chatroomsbuttonbar"
        | "picturesbuttonbar"
        | "videosbuttonbar"
        | "notificationsbuttonbar" => Some(R::ButtonRow(ViewKey::Friends)),
        "mainmenuwindow" => Some(R::SideRail),
        "consoletext" => Some(R::ConsoleLabel),
        // Symbiosis's chat-room name edit: the group DM's name (lead decision 1).
        "roomedit" => Some(R::RoomName),
        "downloadblink" => Some(R::Lamp(LampKey::Files)),
        "gamelogo" => Some(R::GameLogo),
        // The popup's info pane (Space Cowboy's, Total Chaos's and Onyx's
        // `InfoView` component, StealthX's `InfoViewTile`; Classic's
        // `InfoInfoView` component on its `InfoView` tile; Symbiosis's
        // `MainInfoViewContent` component, `Class="InfoView"`,
        // `Components/Popups.xml:83`, on its `MainInfoContent` tile).
        "infoview" | "infoviewtile" | "maininfoviewcontent" => Some(R::InfoView),
        "topbuttonbar"
        | "baseboard"
        | "buttonarea"
        | "tabbuttonarea"
        | "buttonholder"
        | "bottombar"
        | "tabsbuttonscontainer"
        | "centerconsole"
        | "statuswrapper"
        | "statusbarcontainer"
        | "infobar"
        | "content"
        | "mainwindow" => Some(R::Group),
        _ => None,
    }
}

/// The role of a tile named `name` with component `component`, under a
/// parent of role `parent`; `None` for a name the table lacks (an
/// `ImportError` unless the tile is classless art: [`NodeRole::Decor`]).
#[must_use]
pub fn role(
    name: &str,
    component: Option<&str>,
    parent: Option<&Role>,
    f: &Facts<'_>,
) -> Option<Role> {
    use NodeRole as R;
    let shows = |r: NodeRole| Some(Role::Shows(r));
    let parent_shown = parent.map(Role::shown);
    let comp = component.map(key);
    let name_k = key(name);
    // Onyx's bottom `Logo2` is decor, not a second logo (the digits the
    // key drops are its name here).
    if name.trim().eq_ignore_ascii_case("logo2") {
        return shows(R::Decor);
    }
    // A button bar's child named for a view and not for a button is that
    // view's row (Space Cowboy's and Total Chaos's `FriendsButtons`,
    // `ServersButtons`, `FilesButtons` under `TabButtonArea`).
    if parent_shown == Some(R::ButtonBar) && button(&name_k).is_none() {
        if let Some(v) = view(name) {
            return shows(R::ButtonRow(v));
        }
    }
    // A tab control holding the buddy list is the views; one holding
    // button rows is the button bar (Classic names both `TabbedControl`).
    // An INI `Class=TabControl` is the same control. One named for a button
    // (SimpleX's `TabAddButton`: `AddButton`/`AddServerButton`/`SearchFiles`)
    // is one slot of the button row, swapping its button with the tab: a
    // group of buttons. One holding knobs is Combat's rotary tab strip.
    let tabbed_class = f.class.is_some_and(|c| {
        let c = c.trim();
        c.eq_ignore_ascii_case("TabControl") || c.eq_ignore_ascii_case("TabbedControl")
    });
    if f.children.iter().any(|c| key(c) == "knob") && tabbed_class {
        return shows(R::TabStrip);
    }
    // Combat's `[ConsoleText]` is `Class=TabControl` but holds the console
    // plate's per-tab texts (`BuddyText`/`ServerText`/`DownloadText`, the
    // label's parts, lead decision 1), not views.
    if name_k == "consoletext" || comp.as_deref() == Some("consoletext") {
        return shows(R::ConsoleLabel);
    }
    // Combat's `DownloadBlink` is a tab control too (one blinker per state,
    // `WhiteBlinkNoAnimation`/`RedBlinker`): the files lamp, not the views.
    if name_k == "downloadblink" {
        return shows(R::Lamp(LampKey::Files));
    }
    // The info view switch is a tab control too (Combat's `[InfoViewSwitch]`
    // `Class=TabControl`, `combat.ini:786-794`): its two windows
    // (`InfoViewSwitchIn`/`InfoviewSwitchOut`) are the toggle's two states,
    // one shown at a time, so they are its parts, not two buttons in one place.
    if tabbed_class && info_toggle(&name_k) {
        return named(&name_k, f);
    }
    if comp.as_deref() == Some("tabbedcontrol") || name_k == "maintabbedarea" || tabbed_class {
        // StealthX's rail `InfoJoinChatTab` (`Component="ChatTab"`,
        // `Class="TabControl"`, `components.xml:705`; `popups.xml:75-81`)
        // holds `HostChatButton`/`JoinChatButton`/`LeaveChatButton`: one
        // page per chat state, never shown together, so the button bar
        // (its pages), not one view's slot as its "chat"/"info" name says.
        if chat_states(f.children) {
            return shows(R::ButtonBar);
        }
        if button(&name_k).is_some() {
            return shows(R::Group);
        }
        let rows = f.children.iter().any(|c| key(c).contains("button"));
        return shows(if rows { R::ButtonBar } else { R::Views });
    }
    let found = comp
        .as_deref()
        .and_then(|c| named(c, f))
        .or_else(|| named(&name_k, f))
        .or_else(|| match parent_shown {
            Some(R::TabStrip) if name_k == "knob" => {
                let n: usize = name
                    .trim_start_matches(|c: char| !c.is_ascii_digit())
                    .parse()
                    .ok()?;
                // Combat's four knob stops share one place (`combat.ini`
                // `[TabControl]`, all at X15 Y36); the fourth is the return
                // sweep through Servers, a stop like the other three.
                shows(
                    KNOBS
                        .get(n.checked_sub(1)?)
                        .map_or(R::Part, |v| R::Knob(*v)),
                )
            }
            Some(R::TabStrip) => view(name).map(|v| Role::Shows(R::Tab(v))),
            Some(R::ButtonRow(_) | R::ButtonBar | R::Group) => button(&name_k)
                .or_else(|| comp.as_deref().and_then(button))
                .map(click),
            _ => None,
        })
        .or_else(|| {
            tab(&name_k)
                .and_then(|()| view(name))
                .map(|v| Role::Shows(R::Tab(v)))
        })
        .or_else(|| resize(&name_k))
        .or_else(|| comp.as_deref().and_then(resize));
    let found = match found {
        Some(Role::Shows(R::ButtonRow(_))) => {
            view(name).map(|v| Role::Shows(R::ButtonRow(v))).or(found)
        }
        other => other,
    };
    match (found, parent) {
        // The same component inside itself is its piece (`FileMenu` under
        // `XfireMenuText`, `UserNameText` under `UserNameField`, Classic's
        // `TabNormalButton` art under its `TabNormalText` tab); a group or
        // decor inside one is its own.
        (Some(r), Some(p)) if !matches!(r.shown(), R::Group | R::Decor | R::Part) && &r == p => {
            shows(R::Part)
        }
        (Some(r), _) => Some(r),
        (None, Some(p))
            if p.interactive()
                || matches!(
                    p.shown(),
                    R::Logo | R::AdArea | R::View(_) | R::ConsoleLabel | R::Lamp(_) | R::NameField
                ) =>
        {
            shows(R::Part)
        }
        (None, _) => {
            let classless = f.class.is_none_or(|c| {
                ["tile", "image", "bitmap", "background"]
                    .iter()
                    .any(|t| c.trim().eq_ignore_ascii_case(t))
            });
            (f.has_art && classless).then_some(Role::Shows(R::Decor))
        }
    }
}
