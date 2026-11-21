//! The pack-name → role table (plan SK2, design §1 and §5 "Import"):
//! keyed by the tile's `Component=` (an INI `[Section]`), else its
//! `Name=`, ASCII-case-insensitively with trailing digits dropped
//! (Classic's `GameButton1`/`GameButton2`, Combat's `Knob01`).

use rekindle_types::dto::skin_tree::{ActionKey, Edge, LampKey, MenuKey, NodeRole, ViewKey};

/// What a tile is, beyond its names: whether it has art, its class and
/// its children's names.
pub struct Facts<'a> {
    pub has_art: bool,
    pub class: Option<&'a str>,
    pub children: &'a [String],
}

fn key(name: &str) -> String {
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

const KNOBS: [ViewKey; 3] = [ViewKey::Friends, ViewKey::Servers, ViewKey::Files];

fn button(k: &str) -> Option<ActionKey> {
    let rows: [(&[&str], ActionKey); 20] = [
        (&["addserver", "addfav", "favorite"], ActionKey::AddServer),
        (&["startdownload"], ActionKey::StartDownload),
        (&["stopdownload"], ActionKey::StopDownload),
        // Combat's `ChatRefreshStopTab` on the Files tab (lead decision 1).
        (&["stop"], ActionKey::StopDownload),
        // Symbiosis's `ButtonViewPictures`: open the selected shared file.
        (&["viewpicture", "viewvideo"], ActionKey::OpenDownload),
        (&["executedownload", "openfile"], ActionKey::OpenDownload),
        (&["downloadinfo"], ActionKey::DownloadInfo),
        // Symbiosis's Pictures/Videos rows: a shared file's info, and
        // sharing one (the attachment upload) (lead decision 1).
        (&["infopicture", "infovideo"], ActionKey::DownloadInfo),
        (&["upload"], ActionKey::Browse),
        (&["bigbrowsefile", "browse"], ActionKey::Browse),
        (&["find", "search"], ActionKey::Find),
        (&["roomenter", "enterroom", "enter"], ActionKey::EnterRoom),
        (&["roomcreate", "create"], ActionKey::CreateRoom),
        (&["delete"], ActionKey::DeleteItem),
        (&["refresh"], ActionKey::Refresh),
        (&["add"], ActionKey::AddFriend),
        (&["chat", "im"], ActionKey::Chat),
        (&["game", "join", "play"], ActionKey::Join),
        (&["info", "profile"], ActionKey::Profile),
        (&["fav"], ActionKey::Favorite),
    ];
    let k = k
        .trim_start_matches("button")
        .trim_start_matches("tab")
        .trim_end_matches("button");
    rows.into_iter()
        .find_map(|(keys, a)| keys.iter().any(|p| k.starts_with(p)).then_some(a))
}

/// A tab outside a tab strip: Classic's `TabNormalText`/`TabNormalButton`
/// pairs in Onyx's menu bar and Alloy's tab bar, Symbiosis's
/// `ButtonNotificationsTab` beside the name, XGlow's `[Friends]`.
fn tab(k: &str) -> Option<()> {
    let paired = (k.starts_with("tab") && (k.ends_with("text") || k.ends_with("button")))
        || (k.starts_with("button") && k.ends_with("tab"));
    (paired || matches!(k, "friends" | "servers" | "downloads" | "files")).then_some(())
}

fn resize(k: &str) -> Option<NodeRole> {
    let rest = k
        .strip_prefix("mainresizegripper")
        .or_else(|| k.strip_prefix("resize"))
        .or_else(|| k.strip_prefix("gripper"))
        .or_else(|| k.strip_suffix("gripper"))?;
    let edge = match rest {
        "up" | "top" => Edge::Top,
        "down" | "bottom" => Edge::Bottom,
        "left" => Edge::Left,
        "right" => Edge::Right,
        "topleft" | "ul" => Edge::TopLeft,
        "topright" => Edge::TopRight,
        "bottomleft" => Edge::BottomLeft,
        _ => Edge::BottomRight,
    };
    Some(NodeRole::Resize(edge))
}

fn named(k: &str, f: &Facts<'_>) -> Option<NodeRole> {
    use NodeRole as R;
    let menu = |m| Some(R::Menu(m));
    match k {
        "maintitlebar" | "windowdragger" | "combatdragbar" => Some(R::Drag),
        "xfirelogo" | "logo" | "logox" | "wndxfire" => Some(R::Logo),
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
        "minimizebox" | "minimize" | "buttonminimize" | "wndmin" | "minimizebutton" => {
            Some(R::Minimize)
        }
        "maximizebox" | "maximize" | "buttonmaximize" | "wndmax" => Some(R::Maximize),
        "closebox" | "close" | "buttonclose" | "wndclose" => Some(R::Close),
        "xfiremenutext" | "xfiremenu" | "filemenutext" | "filemenu" => menu(MenuKey::First),
        "toolsmenutext" | "toolsmenu" | "toolmenu" => menu(MenuKey::Tools),
        "helpmenutext" | "helpmenu" => menu(MenuKey::Help),
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
        | "thexcover" => Some(R::Decor),
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
        "infoviewbutton" | "expandbutton" | "infoviewswitch" | "ivbuttonholder" | "viewinfo"
        | "buttonmaininfoout" => Some(R::InfoToggle),
        "menubutton" => Some(R::SideRail),
        "consoletext" => Some(R::ConsoleLabel),
        // Symbiosis's chat-room name edit: the group DM's name (lead decision 1).
        "roomedit" => Some(R::RoomName),
        "downloadblink" => Some(R::Lamp(LampKey::Files)),
        "tclink" => Some(R::PackLink),
        "gamelogo" => Some(R::GameLogo),
        "systemmenu" => Some(R::SystemMenu),
        "adddownloadtab" => Some(R::Button(ActionKey::AddFriend)),
        "chatrefreshstoptab" => Some(R::Button(ActionKey::Chat)),
        "joinplaytab" => Some(R::Button(ActionKey::Join)),
        "profilebrowsetab" => Some(R::Button(ActionKey::Profile)),
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
    parent: Option<NodeRole>,
    f: &Facts<'_>,
) -> Option<NodeRole> {
    use NodeRole as R;
    let comp = component.map(key);
    let name_k = key(name);
    // Onyx's bottom `Logo2` is decor, not a second logo (the digits the
    // key drops are its name here).
    if name.trim().eq_ignore_ascii_case("logo2") {
        return Some(R::Decor);
    }
    // A button bar's child named for a view and not for a button is that
    // view's row (Space Cowboy's and Total Chaos's `FriendsButtons`,
    // `ServersButtons`, `FilesButtons` under `TabButtonArea`).
    if parent == Some(R::ButtonBar) && button(&name_k).is_none() {
        if let Some(v) = view(name) {
            return Some(R::ButtonRow(v));
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
        return Some(R::TabStrip);
    }
    // Combat's `[ConsoleText]` is `Class=TabControl` but holds the console
    // plate's per-tab texts (`BuddyText`/`ServerText`/`DownloadText`, the
    // label's parts, lead decision 1), not views.
    if name_k == "consoletext" || comp.as_deref() == Some("consoletext") {
        return Some(R::ConsoleLabel);
    }
    // Combat's `DownloadBlink` is a tab control too (one blinker per state,
    // `WhiteBlinkNoAnimation`/`RedBlinker`): the files lamp, not the views.
    if name_k == "downloadblink" {
        return Some(R::Lamp(LampKey::Files));
    }
    // The info view switch is a tab control too (Combat's `[InfoViewSwitch]`
    // `Class=TabControl`, `combat.ini:786-794`): its two windows
    // (`InfoViewSwitchIn`/`InfoviewSwitchOut`) are the toggle's two states,
    // one shown at a time, so they are its parts, not two buttons in one place.
    if tabbed_class && named(&name_k, f) == Some(R::InfoToggle) {
        return Some(R::InfoToggle);
    }
    if comp.as_deref() == Some("tabbedcontrol") || name_k == "maintabbedarea" || tabbed_class {
        if button(&name_k).is_some() {
            return Some(R::Group);
        }
        let rows = f.children.iter().any(|c| key(c).contains("button"));
        return Some(if rows { R::ButtonBar } else { R::Views });
    }
    let found = comp
        .as_deref()
        .and_then(|c| named(c, f))
        .or_else(|| named(&name_k, f))
        .or_else(|| match parent {
            Some(R::TabStrip) if name_k == "knob" => {
                let n: usize = name
                    .trim_start_matches(|c: char| !c.is_ascii_digit())
                    .parse()
                    .ok()?;
                // Combat's four knob rotations share one place
                // (`combat.ini` `[TabControl]`, all at X15 Y36) for three
                // tabs: the fourth is a piece of the strip's art.
                Some(
                    KNOBS
                        .get(n.checked_sub(1)?)
                        .map_or(R::Part, |v| R::Knob(*v)),
                )
            }
            Some(R::TabStrip) => view(name).map(R::Tab),
            Some(R::ButtonRow(_) | R::ButtonBar | R::Group) => button(&name_k)
                .or_else(|| comp.as_deref().and_then(button))
                .map(R::Button),
            _ => None,
        })
        .or_else(|| tab(&name_k).and_then(|()| view(name)).map(R::Tab))
        .or_else(|| resize(&name_k))
        .or_else(|| comp.as_deref().and_then(resize));
    let found = match found {
        Some(R::ButtonRow(_)) => view(name).map(R::ButtonRow).or(found),
        other => other,
    };
    match (found, parent) {
        // The same component inside itself is its piece (`FileMenu` under
        // `XfireMenuText`, `UserNameText` under `UserNameField`, Classic's
        // `TabNormalButton` art under its `TabNormalText` tab); a group or
        // decor inside one is its own.
        (Some(r), Some(p)) if !matches!(r, R::Group | R::Decor | R::Part) && r == p => {
            Some(R::Part)
        }
        (Some(r), _) => Some(r),
        (None, Some(p))
            if p.interactive()
                || matches!(
                    p,
                    R::Logo
                        | R::AdArea
                        | R::View(_)
                        | R::ConsoleLabel
                        | R::Lamp(_)
                        | R::NameField
                        | R::InfoToggle
                ) =>
        {
            Some(R::Part)
        }
        (None, _) => {
            let classless = f.class.is_none_or(|c| {
                ["tile", "image", "bitmap", "background"]
                    .iter()
                    .any(|t| c.trim().eq_ignore_ascii_case(t))
            });
            (f.has_art && classless).then_some(R::Decor)
        }
    }
}
