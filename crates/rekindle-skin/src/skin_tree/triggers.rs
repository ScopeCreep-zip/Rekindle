//! A tile's `<Trigger Type="E">A</Trigger>` pairs read into typed
//! [`NodeTrigger`]s (plan SK2-A, lead decision 36; design `sk2a/design.md`
//! §2.3). The INI era's `Action.<event>=A` and `<state>.Trigger=<signal>`
//! keys reach here as the same elements (`xfire_ini::title::component`).
//! Event and action names are the engine's (`Xfire.exe` UTF-16 tables,
//! design §2.1), matched ASCII-case-insensitively as the client does
//! (Classic writes `AppLaunchURL`, Total Chaos `AppLaunchUrl`). A name
//! outside the closed vocabulary refuses the pack
//! ([`ImportError::UnknownAction`]): nothing is dropped.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_types::dto::skin_action::{
    AnimateSpec, AppSignal, Capability, EngineMenu, EngineView, NodeState, NodeTrigger,
    OverlayPart, PackUrl, PopupRef, SkinAction, SkinEvent, SoundRef, TileRef,
};
use rekindle_types::dto::skin_tree_role::Edge;

use super::import::bad;
use crate::import::tree::Node;
use crate::import::ImportError;

/// What a trigger's action may need from the pack beyond its own text.
pub(super) struct Pack<'a> {
    /// The tile names of the pack's `<Popups>` roots, lowercased (empty
    /// for a pack with none: the INI era's popups are skin files).
    pub popups: &'a BTreeSet<String>,
    /// The control groups the pack's components declare.
    pub groups: &'a Groups,
}

/// The control groups a pack's components declare.
#[derive(Clone)]
pub(super) struct Groups {
    /// Every `GroupId`/`ControlGroupId`.
    pub all: BTreeSet<u16>,
    /// Every `ControlGroupId`: the groups with a paged control, whose pages
    /// a `GroupMessage(n)` selects.
    pub controls: BTreeSet<u16>,
}

impl Groups {
    /// The groups `components` (by name) declare.
    pub(super) fn of(components: &BTreeMap<String, &Node>) -> Self {
        let ids = |keys: &[&str]| -> BTreeSet<u16> {
            components
                .values()
                .flat_map(|c| keys.iter().filter_map(|k| c.attr_ci(k)))
                .filter_map(|g| g.trim().parse().ok())
                .collect()
        };
        Self {
            all: ids(&["GroupId", "ControlGroupId"]),
            controls: ids(&["ControlGroupId"]),
        }
    }
}

/// Who declared the trigger: the tile, or its component.
struct Owner<'a> {
    tile: &'a Node,
    comp: Option<&'a Node>,
    id: &'a str,
    file: String,
}

/// `tile`'s triggers then its component's (design §2.3: tile first).
///
/// # Errors
/// An unknown event or action ([`ImportError::UnknownAction`]), or an
/// argument the action cannot take.
pub(super) fn read(
    tile: &Node,
    comp: Option<&Node>,
    id: &str,
    pack: &Pack<'_>,
) -> Result<Vec<NodeTrigger>, ImportError> {
    let mut out = Vec::new();
    let comp_name = comp.and_then(|c| c.attr_ci("Name")).unwrap_or("-");
    for (src, file) in [
        (Some(tile), format!("skin tree (tile {id})")),
        (comp, format!("skin tree (component {comp_name})")),
    ] {
        let Some(src) = src else { continue };
        let owner = Owner {
            tile,
            comp,
            id,
            file,
        };
        for t in own_triggers(src) {
            let (event, action) = (t.attr_ci("Type").unwrap_or_default().trim(), t.text.trim());
            // Onyx's `<Trigger Type="OnClick"></Trigger>` (`components.xml:2089`)
            // names no action: there is nothing to run, and no name to refuse.
            if action.is_empty() || radio(action, &owner, pack) {
                continue;
            }
            out.push(NodeTrigger {
                on: event_of(event).ok_or_else(|| unknown(&owner, event))?,
                run: action_of(action, &owner, pack)?,
            });
        }
    }
    Ok(out)
}

/// `node`'s own `<Triggers><Trigger>` (and bare `<Trigger>`) children,
/// never a nested tile's.
fn own_triggers(node: &Node) -> impl Iterator<Item = &Node> {
    node.children
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case("Triggers"))
        .flat_map(|c| c.children.iter())
        .chain(node.children.iter())
        .filter(|c| c.name.eq_ignore_ascii_case("Trigger"))
}

fn unknown(owner: &Owner<'_>, name: &str) -> ImportError {
    ImportError::UnknownAction {
        file: owner.file.clone(),
        tile: owner.id.to_owned(),
        name: name.to_owned(),
    }
}

fn view_of(name: &str) -> Option<EngineView> {
    use EngineView as V;
    Some(match name {
        "home" => V::Home,
        "friends" => V::Friends,
        "chats" => V::Chats,
        "chatrooms" => V::ChatRooms,
        "servers" => V::Servers,
        "files" => V::Files,
        "videos" => V::Videos,
        "screenshots" => V::Screenshots,
        "notifications" => V::Notifications,
        "options" => V::Options,
        "help" => V::Help,
        "inviteuserstoroom" => V::InviteUsersToRoom,
        _ => return None,
    })
}

fn capability_of(name: &str) -> Option<Capability> {
    use Capability as C;
    Some(match name {
        "addfriend" => C::AddFriend,
        "chat" => C::Chat,
        "profile" => C::Profile,
        "join" => C::Join,
        "voicechat" => C::VoiceChat,
        "hostvoicechat" => C::HostVoiceChat,
        "leavevoicechat" => C::LeaveVoiceChat,
        "cancelvoicechat" => C::CancelVoiceChat,
        "muteself" => C::MuteSelf,
        "unmuteself" => C::UnmuteSelf,
        "addserver" => C::AddServer,
        "refreshserver" => C::RefreshServer,
        "startdownload" => C::StartDownload,
        "stopdownload" => C::StopDownload,
        "executedownload" => C::ExecuteDownload,
        "hostgamelobby" => C::HostGameLobby,
        "joingamelobby" => C::JoinGameLobby,
        "launchgamelobby" => C::LaunchGameLobby,
        "leavegamelobby" => C::LeaveGameLobby,
        "createroom" => C::CreateRoom,
        "enterroom" => C::EnterRoom,
        "delete" => C::Delete,
        "deleteselected" => C::DeleteSelected,
        "openselected" => C::OpenSelected,
        "uploadselected" => C::UploadSelected,
        _ => return None,
    })
}

fn part_of(name: &str) -> Option<OverlayPart> {
    use OverlayPart as P;
    Some(match name {
        "fps" => P::Fps,
        "clock" => P::Clock,
        "alert" => P::Alert,
        "webcam" => P::WebCam,
        "voice" | "voiceoverlay" => P::VoiceOverlay,
        "browser" => P::Browser,
        _ => return None,
    })
}

fn signal_of(e: &str) -> Option<AppSignal> {
    use AppSignal as S;
    if let Some(c) = e.strip_prefix("appcannot") {
        return capability_of(c).map(S::Cannot);
    }
    if let Some(c) = e.strip_prefix("appcan") {
        return capability_of(c).map(S::Can);
    }
    if let Some(p) = e.strip_prefix("pref") {
        if let Some(p) = p.strip_suffix("enabled") {
            return part_of(p).map(S::PrefEnabled);
        }
        return p
            .strip_suffix("disabled")
            .and_then(part_of)
            .map(S::PrefDisabled);
    }
    Some(match e {
        "activeusertyping" => S::ActiveUserTyping,
        "activeusernottyping" => S::ActiveUserNotTyping,
        "appvoiceringing" => S::VoiceRinging,
        "appuserstartedtalking" => S::UserStartedTalking,
        "appuserstoppedtalking" => S::UserStoppedTalking,
        "incomingvoicechat" => S::IncomingVoiceChat,
        "outgoingvoicechat" => S::OutgoingVoiceChat,
        "startvoicechat" => S::StartVoiceChat,
        "endvoicechat" => S::EndVoiceChat,
        "speakeron" => S::SpeakerOn,
        "speakeroff" => S::SpeakerOff,
        "pushtotalkon" => S::PushToTalkOn,
        "pushtotalkoff" => S::PushToTalkOff,
        "newnotification" => S::NewNotification,
        "notificationsvisited" => S::NotificationsVisited,
        "downloadcomplete" => S::DownloadComplete,
        "xfireurldownload" => S::XfireUrlDownload,
        "disconnected" => S::Disconnected,
        "appstartinglogin" => S::StartingLogin,
        "appstartrefreshservers" => S::StartRefreshServers,
        "appstoprefreshservers" => S::StopRefreshServers,
        "gameprefbrowserdisabled" => S::GamePrefBrowserDisabled,
        "enablewebcamoverlay" => S::WebCamOverlayEnabled,
        "disablewebcamoverlay" => S::WebCamOverlayDisabled,
        _ => return None,
    })
}

/// An engine event: `On` + the engine word (`OnLeftButtonDown`, INI
/// `lbtndown`, `onfilesfocus`, `AppCanJoin`).
pub(super) fn event_of(raw: &str) -> Option<SkinEvent> {
    use SkinEvent as E;
    let lower = raw.to_ascii_lowercase();
    let e = lower.strip_prefix("on").unwrap_or(&lower);
    Some(match e {
        "click" => E::Click,
        "doubleclick" => E::DoubleClick,
        "leftbuttondown" | "lbtndown" => E::LeftDown,
        "rightbuttondown" | "rbtndown" => E::RightDown,
        "rightclick" | "rclick" => E::RightClick,
        "activate" => E::Activate,
        "deactivate" => E::Deactivate,
        "activemode" => E::ActiveMode,
        "inactivemode" => E::InactiveMode,
        "tabactivated" => E::TabActivated,
        "checkboxchecked" => E::CheckboxChecked,
        "checkboxunchecked" => E::CheckboxUnchecked,
        "tacked" => E::Tacked,
        "untacked" => E::Untacked,
        "filesfocus" => E::FilesFocus,
        "fileskillfocus" => E::FilesKillFocus,
        _ => {
            if let Some(v) = e.strip_prefix("show").and_then(view_of) {
                return Some(E::Show(v));
            }
            return signal_of(e).map(E::App);
        }
    })
}

/// `Name` or `Name(args)`.
fn split(raw: &str) -> (String, Option<&str>) {
    match raw.split_once('(') {
        Some((n, rest)) => (
            n.trim().to_ascii_lowercase(),
            Some(rest.trim_end().trim_end_matches(')').trim()),
        ),
        None => (raw.trim().to_ascii_lowercase(), None),
    }
}

fn plain(name: &str) -> Option<SkinAction> {
    use SkinAction as A;
    Some(match name {
        "appdragwindow" => A::DragWindow,
        "appminimize" => A::Minimize,
        "appmaximize" => A::Maximize,
        "appclose" => A::Close,
        "resizet" => A::Resize(Edge::Top),
        "resizeb" => A::Resize(Edge::Bottom),
        "resizel" => A::Resize(Edge::Left),
        "resizer" => A::Resize(Edge::Right),
        "resizeul" => A::Resize(Edge::TopLeft),
        "resizeur" => A::Resize(Edge::TopRight),
        "resizebl" => A::Resize(Edge::BottomLeft),
        "resizebr" => A::Resize(Edge::BottomRight),
        "menufile" => A::Menu(EngineMenu::File),
        "menutool" => A::Menu(EngineMenu::Tool),
        "menuhelp" => A::Menu(EngineMenu::Help),
        "menustatus" => A::Menu(EngineMenu::Status),
        "menusys" | "menusystem" => A::Menu(EngineMenu::Sys),
        "menucwactions" => A::Menu(EngineMenu::CwActions),
        "menucwhelp" => A::Menu(EngineMenu::CwHelp),
        "menugcsystem" => A::Menu(EngineMenu::GcSystem),
        "menugcactions" => A::Menu(EngineMenu::GcActions),
        "menugchelp" => A::Menu(EngineMenu::GcHelp),
        "contextmenusys" => A::ContextMenu(EngineMenu::Sys),
        "animatestop" => A::AnimateStop,
        "addfriend" => A::AddFriend,
        "chatfriend" => A::ChatFriend,
        "profilefriend" => A::ProfileFriend,
        "viewprofile" => A::ViewProfile,
        "joinfriend" => A::JoinFriend,
        "sendmessage" => A::SendMessage,
        "appvoicechat" => A::VoiceChat,
        "apphostvoicechat" => A::HostVoiceChat,
        "appleavevoicechat" => A::LeaveVoiceChat,
        "appgroupvoicechat" => A::GroupVoiceChat,
        "apphostgroupvoicechat" => A::HostGroupVoiceChat,
        "appleavegroupvoicechat" => A::LeaveGroupVoiceChat,
        "appcancelvoicechat" => A::CancelVoiceChat,
        "endvoicechat" => A::EndVoiceChat,
        "appmuteself" => A::MuteSelf,
        "appunmuteself" => A::UnmuteSelf,
        "showvoicechatwindow" => A::ShowVoiceWindow,
        "hostgamelobby" => A::HostGameLobby,
        "joingamelobby" => A::JoinGameLobby,
        "leavegamelobby" => A::LeaveGameLobby,
        "launchgamelobby" => A::LaunchGameLobby,
        "addserver" => A::AddServer,
        "refreshserver" => A::RefreshServer,
        "startdownload" => A::StartDownload,
        "stopdownload" => A::StopDownload,
        "executedownload" => A::OpenDownload,
        "downloadinfo" => A::DownloadInfo,
        "newroom" => A::NewRoom,
        "createroom" => A::CreateRoom,
        "enterroom" => A::EnterRoom,
        "enterattachedroom" => A::EnterAttachedRoom,
        "joinroom" => A::JoinRoom,
        "inviteuserstoroom" => A::InviteUsersToRoom,
        "deleteselected" => A::DeleteSelected,
        "openselected" => A::OpenSelected,
        "uploadselected" => A::UploadSelected,
        "searchfor" => A::SearchFor,
        "showwindow" => A::ShowWindow,
        "hidewindow" => A::HideWindow,
        "dialogboxok" => A::DialogOk,
        "dialogboxcancel" => A::DialogCancel,
        "updateclock" => A::UpdateClock,
        "tackwindow" => A::TackWindow,
        "untackwindow" => A::UntackWindow,
        "openoverlayoptionsdlg" => A::OpenOverlayOptions,
        "attachedclose" => A::AttachedClose,
        "browseback" => A::BrowseBack,
        "browseforward" => A::BrowseForward,
        _ => {
            if let Some(p) = name.strip_prefix("enable").and_then(part_of) {
                return Some(A::EnableOverlay(p));
            }
            if let Some(p) = name.strip_prefix("disable").and_then(part_of) {
                return Some(A::DisableOverlay(p));
            }
            return name.strip_prefix("show").and_then(view_of).map(A::ShowView);
        }
    })
}

/// A component's (or tile's) control group: `GroupId`, else
/// `ControlGroupId`.
fn own_group(owner: &Owner<'_>) -> Option<u16> {
    [owner.comp, Some(owner.tile)]
        .into_iter()
        .flatten()
        .find_map(|n| n.attr_ci("GroupId").or(n.attr_ci("ControlGroupId")))
        .and_then(|g| g.trim().parse().ok())
}

/// Whether `action` is a sticky push button's radio message: a one-argument
/// `GroupMessage(n)` naming the button's own `GroupId` (Symbiosis's tabs,
/// `GroupId="8765"` with `OnLeftButtonDown` `GroupMessage(8765)`,
/// `MainWindow.xml:910-915`) when no component declares a paged control of
/// that group (`ControlGroupId`), which tells the other sticky buttons of
/// the group to release. It selects no page of any group (the group has no
/// paged control, so no page 8765), and the shell's shown-tab state already
/// is the radio (one tab down, the shown view's), so it imports as no
/// trigger. A group with a paged control has its pages: Onyx's Servers tab
/// (`GroupId="1"`, `GroupMessage(1)`, `components.xml:840-846`, its views
/// `ControlGroupId="1"`) and Combat's `[Knob02]` (`GroupId=2`,
/// `GroupMessage(2)`, its knob strip `ControlGroupId=2`, `combat.ini:319,
/// 358-366`) select page `n` of their group, as any `GroupMessage(n)` does.
fn radio(action: &str, owner: &Owner<'_>, pack: &Pack<'_>) -> bool {
    let (name, args) = split(action);
    name == "groupmessage"
        && args.is_some_and(|a| {
            !a.contains(',')
                && a.trim().parse::<u16>().ok().is_some_and(|n| {
                    Some(n) == own_group(owner) && !pack.groups.controls.contains(&n)
                })
        })
}

fn number(s: &str, owner: &Owner<'_>, raw: &str) -> Result<u16, ImportError> {
    s.trim()
        .parse()
        .map_err(|_| bad(format!("{}: {raw}: {s} is not a number", owner.id)))
}

/// A `key=n` of an action's arguments, found by name (Total Chaos writes
/// `Animate(Type=RepeatDelay=4Repeat=-1)`, no commas).
fn arg_value(lower: &str, key: &str) -> Option<i32> {
    lower.split(',').find_map(|part| {
        let at = part.find(&format!("{key}="))? + key.len() + 1;
        let digits: String = part[at..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        digits.parse().ok()
    })
}

/// `Animate(Type=Repeat,Delay=4,Repeat=-1,Action=1)` (and the INI
/// `AnimationStart(GroupId=100,Type=Repeat,…)`'s spec).
fn animate_spec(args: &str, owner: &Owner<'_>, raw: &str) -> Result<AnimateSpec, ImportError> {
    let lower = args.to_ascii_lowercase();
    let value = |key: &str| -> Option<i32> {
        if key == "repeat" {
            // `Type=Repeat` is no value: the count is the `Repeat=` with digits.
            return lower
                .match_indices("repeat=")
                .find_map(|(i, _)| arg_value(&lower[i..], key));
        }
        let at = lower.find(&format!("{key}="))? + key.len() + 1;
        let digits: String = lower[at..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        digits.parse().ok()
    };
    if !lower.contains("type=repeat") {
        return Err(unknown(owner, raw));
    }
    let delay = value("delay").ok_or_else(|| bad(format!("{}: {raw}: no Delay", owner.id)))?;
    let to_u16 = |v: i32| u16::try_from(v).map_err(|_| bad(format!("{}: {raw}", owner.id)));
    Ok(AnimateSpec {
        delay: to_u16(delay)?,
        repeat: value("repeat")
            .filter(|r| *r >= 0)
            .map(to_u16)
            .transpose()?,
        action: value("action").map(to_u16).transpose()?,
    })
}

/// An INI `AnimationStart`/`AnimationStop`'s `GroupId=`.
fn group_arg(args: &str, owner: &Owner<'_>, raw: &str) -> Result<u16, ImportError> {
    let g = arg_value(&args.to_ascii_lowercase(), "groupid")
        .and_then(|g| u16::try_from(g).ok())
        .ok_or_else(|| bad(format!("{}: {raw}: no GroupId", owner.id)))?;
    Ok(g)
}

/// The popup a `LaunchSkin` opens: the component's (else the tile's)
/// `<Popup>`, which must name a tile of the pack's `<Popups>` when it has
/// any (design §2.3).
fn popup(owner: &Owner<'_>, pack: &Pack<'_>, raw: &str) -> Result<SkinAction, ImportError> {
    let name = [owner.comp, Some(owner.tile)]
        .into_iter()
        .flatten()
        .find_map(|n| n.children_named("Popup").next())
        .map(|p| p.text.trim().to_owned())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| bad(format!("{}: {raw} names no <Popup>", owner.id)))?;
    if !pack.popups.is_empty() && !pack.popups.contains(&name.to_ascii_lowercase()) {
        return Err(bad(format!(
            "{}: {raw} launches {name}, not a tile of the pack's <Popups>",
            owner.id
        )));
    }
    Ok(SkinAction::LaunchPopup(PopupRef { popup: name }))
}

fn action_of(raw: &str, owner: &Owner<'_>, pack: &Pack<'_>) -> Result<SkinAction, ImportError> {
    let (name, args) = split(raw);
    let need = || {
        args.filter(|a| !a.is_empty())
            .ok_or_else(|| unknown(owner, raw))
    };
    match name.as_str() {
        "state" => match need()?.to_ascii_lowercase().as_str() {
            "normal" => Ok(SkinAction::SetState(NodeState::Normal)),
            "disabled" => Ok(SkinAction::SetState(NodeState::Disabled)),
            "down" => Ok(SkinAction::SetState(NodeState::Down)),
            _ => Err(unknown(owner, raw)),
        },
        "groupmessage" => {
            let a = need()?;
            let (page, group) = match a.split_once(',') {
                Some((p, g)) => (number(p, owner, raw)?, number(g, owner, raw)?),
                None => (
                    number(a, owner, raw)?,
                    own_group(owner)
                        .ok_or_else(|| bad(format!("{}: {raw} in no group", owner.id)))?,
                ),
            };
            if !pack.groups.all.contains(&group) {
                return Err(bad(format!("{}: {raw}: no group {group}", owner.id)));
            }
            Ok(SkinAction::GroupPage { group, page })
        }
        "showtile" | "hidetile" => {
            let t = args
                .filter(|a| !a.is_empty())
                .map_or(TileRef::This, |a| TileRef::Named(a.to_owned()));
            Ok(if name == "showtile" {
                SkinAction::ShowTile(t)
            } else {
                SkinAction::HideTile(t)
            })
        }
        "launchskin" => popup(owner, pack, raw),
        "animate" => Ok(SkinAction::Animate(animate_spec(need()?, owner, raw)?)),
        "animationstart" => {
            let a = need()?;
            Ok(SkinAction::AnimationStart {
                group: group_arg(a, owner, raw)?,
                spec: animate_spec(a, owner, raw)?,
            })
        }
        "animationstop" => Ok(SkinAction::AnimationStop {
            group: group_arg(need()?, owner, raw)?,
        }),
        "applaunchurl" => Ok(SkinAction::LaunchUrl(PackUrl {
            url: need()?.to_owned(),
        })),
        "playsound" => Ok(SkinAction::PlaySound(SoundRef {
            sound: need()?.to_owned(),
        })),
        _ if args.is_some() => Err(unknown(owner, raw)),
        _ => plain(&name).ok_or_else(|| unknown(owner, raw)),
    }
}
