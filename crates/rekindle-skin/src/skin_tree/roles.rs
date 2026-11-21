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
    let rows: [(&[&str], ActionKey); 16] = [
        (&["addserver", "addfav", "favorite"], ActionKey::AddServer),
        (&["startdownload"], ActionKey::StartDownload),
        (&["stopdownload"], ActionKey::StopDownload),
        (&["executedownload", "openfile"], ActionKey::OpenDownload),
        (&["downloadinfo"], ActionKey::DownloadInfo),
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
        "adarea" | "adbar" => Some(R::AdArea),
        "tabheader" | "tabs" | "tabbuttonbar" => Some(R::TabStrip),
        "tabcontrol" if f.children.iter().any(|c| key(c) == "knob") => Some(R::TabStrip),
        "buddyview" | "xfireview" => Some(R::View(ViewKey::Friends)),
        "serverview" => Some(R::View(ViewKey::Servers)),
        "downloadview" => Some(R::View(ViewKey::Files)),
        "buttonrow" | "friendsbuttonbar" | "serversbuttonbar" | "filesbuttonbar"
        | "chatroomsbuttonbar" | "picturesbuttonbar" | "videosbuttonbar" => {
            Some(R::ButtonRow(ViewKey::Friends))
        }
        "infoviewbutton" | "expandbutton" | "infoviewswitch" | "ivbuttonholder" | "viewinfo"
        | "buttonmaininfoout" => Some(R::InfoToggle),
        "menubutton" => Some(R::SideRail),
        "consoletext" => Some(R::ConsoleLabel),
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
    // A tab control holding the buddy list is the views; one holding
    // button rows is the button bar (Classic names both `TabbedControl`).
    if comp.as_deref() == Some("tabbedcontrol") || name_k == "maintabbedarea" {
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
                KNOBS.get(n.checked_sub(1)?).map(|v| R::Knob(*v))
            }
            Some(R::TabStrip) => view(name).map(R::Tab),
            Some(R::ButtonRow(_) | R::ButtonBar | R::Group) => button(&name_k)
                .or_else(|| comp.as_deref().and_then(button))
                .map(R::Button),
            _ => None,
        })
        .or_else(|| resize(&name_k))
        .or_else(|| comp.as_deref().and_then(resize));
    let found = match found {
        Some(R::ButtonRow(_)) => view(name).map(R::ButtonRow).or(found),
        other => other,
    };
    match (found, parent) {
        // The same component inside itself is its piece (`FileMenu` under
        // `XfireMenuText`, `UserNameText` under `UserNameField`).
        (Some(r), Some(p)) if !r.repeats() && r == p => Some(R::Part),
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
