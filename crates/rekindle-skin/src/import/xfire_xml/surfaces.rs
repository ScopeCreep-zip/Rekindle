//! The buddy-list and chat windows' surfaces of an Xfire skin (plan
//! E7.7.7 S1, S2): the component (XML) or section (INI) that draws each
//! part of Xfire's main and chat windows, onto the region slot of the
//! Rekindle surface that does the same job.
//!
//! * A bar (`Tile` with `TopLeft`/`Top`/`TopRight`, Classic v3's and the
//!   INI era's `left`/`middle`/`right`, or the list header's
//!   `HeaderLeft`/`HeaderMiddle`/`HeaderRight`) is its three pieces side by
//!   side, `nine-slice` with the end pieces as insets: Xfire tiles the
//!   middle and keeps the ends, as `border-image` does.
//! * A push button's `Middle` image per state. Xfire draws it at its own
//!   size (`MinWidth` = `MaxWidth`); our tab, action and call buttons are
//!   sized by their labels, so it is a `nine-slice` with [`BUTTON_INSET`]
//!   px edges. Its label or icon is baked into the centre, which the slot's
//!   backing covers (plan E7.7.1: a region never replaces a label), so
//!   only the bevel shows.
//! * A container's one `Middle` tile (Symbiosis's chat edit background) is
//!   `stretch`.
//! * Xfire's tab buttons are `PushType="Sticky"` in one `GroupId`
//!   (Symbiosis `Components/MainWindow.xml:910`): the chosen tab keeps its
//!   Down image, so Down also fills `selected` (Classic v3's
//!   `TabNormalButton`: Normal is the tab off, Down on).
//! * Xfire's call button is three components, not states: call, leave
//!   (connected) and answer (ringing). Rekindle's chat window shows a call
//!   button and, while a call with the peer rings out or is connected, a
//!   hang-up button: `chat.call-button` and `chat.call-button-active`. An
//!   incoming call rings in the call window, so the answer image has no
//!   surface here.
//! * The first component bearing a name fills its slot (document order,
//!   as [`super::controls`] does).
//!
//! Not mapped, each with its reason in the inventory (plan E7.7.7): the
//! tab strip (our tabs fill the bar), the Xfire logo launcher (a trademark
//! that opens xfire.com), the ad view and partner graphic, Combat's tab
//! knobs (a dial of four baked images, no tab shape), the send buttons
//! (Rekindle has none: Enter sends), the chat menus, and the other tabs'
//! and bars' buttons (one image per slot).

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, RegionFit, RegionState};

use super::super::{Control, ImportError};
use super::controls::{count_sets, set_name as state_name, three, StateImages, StateTally};

/// A button image's edge kept at 1:1, px: Symbiosis's tab and friends
/// buttons have a 3 px bevel inside a 1 px outline.
pub const BUTTON_INSET: u16 = 4;

/// How a surface's images are read.
#[derive(Debug, Clone, Copy)]
enum Pieces {
    /// Three pieces side by side.
    Row([&'static str; 3]),
    /// One push button `Middle` image.
    Button,
    /// One `Middle` tile behind a container, `stretch`.
    Fill,
}

/// Which skin file a surface's component is read in. The XML era names
/// every window's components apart (`ChatButtonCall`, `GroupChatUsersList`);
/// the INI era reuses section names across its window files (Combat's
/// `[GCMenuBar]` is in both `CombatChatSkin.ini` and
/// `CombatGroupChatSkin.ini`), so its chat window surfaces are read only
/// in the file the skin names for that window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// The main skin file (INI) or any component file (XML).
    Main,
    /// The INI skin's `ChatSkin=` file.
    Chat,
    /// The INI skin's `GroupChatSkin=` file.
    GroupChat,
}

/// A component (XML) or section (INI) whose images fill a surface slot.
#[derive(Debug, Clone, Copy)]
struct Surface {
    name: &'static str,
    region: &'static str,
    pieces: Pieces,
    /// A sticky tab: its Down image also fills `selected`.
    selected_from_down: bool,
    window: Window,
}

const fn surface(name: &'static str, region: &'static str, pieces: Pieces) -> Surface {
    Surface {
        name,
        region,
        pieces,
        selected_from_down: false,
        window: Window::Main,
    }
}

impl Surface {
    const fn sticky(self) -> Self {
        Self {
            selected_from_down: true,
            ..self
        }
    }

    const fn in_window(self, window: Window) -> Self {
        Self { window, ..self }
    }
}

const TOP: Pieces = Pieces::Row(["TopLeft", "Top", "TopRight"]);
/// Classic v3's and the INI era's bars (`left`/`middle`/`right`).
const LMR: Pieces = Pieces::Row(["Left", "Middle", "Right"]);
const HEADER: Pieces = Pieces::Row(["HeaderLeft", "HeaderMiddle", "HeaderRight"]);

/// Component or section name → slot: Symbiosis's, then Classic v3's
/// (`xfirecomponents.xml`, plan E7.7.7 S2), then the INI era's (Combat's
/// `combat.ini` and its chat skin files). A name two families share
/// (`DownArrow`, `BuddyView`) is listed once.
const SURFACES: [Surface; 25] = [
    // Symbiosis, the buddy list (S1).
    surface("StatusBarContainer", "statusbar", TOP),
    surface("ButtonStatusChanger", "statusbar.arrow", Pieces::Button),
    surface("MenuBackground", "buddylist.menubar", TOP),
    surface("InfoBar", "buddylist.infobar", TOP),
    surface("ButtonFriendsTab", "buddylist.tab", Pieces::Button).sticky(),
    surface("XfireView", "buddylist.group-header", HEADER),
    surface("FriendsButtonBar", "buddylist.actionbar", TOP),
    surface("ButtonAddFriend", "buddylist.action-button", Pieces::Button),
    // Symbiosis, the chat windows (`ChatWindow.xml`, `GroupChatWindow.xml`).
    surface("ChatEditContainer", "chat.input", Pieces::Fill),
    surface("ChatButtonBar", "chat.toolbar", TOP),
    surface("ChatButtonCall", "chat.call-button", Pieces::Button),
    surface(
        "ChatButtonLeaveCall",
        "chat.call-button-active",
        Pieces::Button,
    ),
    surface("GroupChatUsersList", "chat.group-header", HEADER),
    // Classic v3 (and Combat for `DownArrow` and `BuddyView`).
    surface("StatusWrapper", "statusbar", LMR),
    surface("DownArrow", "statusbar.arrow", Pieces::Button),
    surface("MenuBar", "buddylist.menubar", LMR),
    surface("UserNameField", "buddylist.infobar", LMR),
    surface("TabNormalButton", "buddylist.tab", LMR).sticky(),
    surface("BuddyView", "buddylist.group-header", HEADER),
    surface("BottomBar", "buddylist.actionbar", LMR),
    surface("AddButton", "buddylist.action-button", Pieces::Button),
    surface("ChatBottomBar", "chat.toolbar", LMR),
    surface("InfoHostChatButton", "chat.call-button", Pieces::Button),
    surface(
        "InfoLeaveChatButton",
        "chat.call-button-active",
        Pieces::Button,
    ),
    surface("GroupChatInfoView", "chat.group-header", HEADER),
];

/// The INI era's own names (Combat): its name bar, add button and the
/// chat windows' menu bars.
const INI_SURFACES: [Surface; 4] = [
    surface("CombatDragBar", "buddylist.infobar", LMR),
    surface("AddFriendButton", "buddylist.action-button", Pieces::Button),
    surface("GCMenuBar", "chat.toolbar", LMR).in_window(Window::Chat),
    surface("GCMenuBar", "chat.group-header", LMR).in_window(Window::GroupChat),
];

/// Whether a piece type is one a surface reads (ASCII-case-insensitive),
/// for the INI `NOTICE`'s read keys.
#[must_use]
pub fn is_read_piece(ty: &str) -> bool {
    [TOP, LMR, HEADER].iter().any(|p| match p {
        Pieces::Row(types) => types.iter().any(|t| t.eq_ignore_ascii_case(ty)),
        Pieces::Button | Pieces::Fill => false,
    })
}

/// The fit of a one-image surface: a nine-slice of [`BUTTON_INSET`]
/// edges for a button, or `stretch` for the status arrow (no label sizes
/// it) and a container's tile.
fn button_fit(region: &str, pieces: Pieces) -> RegionFit {
    if region == "statusbar.arrow" || matches!(pieces, Pieces::Fill) {
        return RegionFit::Stretch;
    }
    RegionFit::NineSlice(Insets {
        top: BUTTON_INSET,
        right: BUTTON_INSET,
        bottom: BUTTON_INSET,
        left: BUTTON_INSET,
    })
}

/// One state's image of `pieces`, `None` when the set lacks a piece.
fn image<S: StateImages>(
    set: &S,
    pieces: Pieces,
) -> Result<Option<(RgbaImage, Option<RegionFit>)>, ImportError> {
    match pieces {
        Pieces::Button | Pieces::Fill => Ok(set.piece("Middle", false)?.map(|img| (img, None))),
        Pieces::Row(types) => {
            let mut row = Vec::with_capacity(3);
            for ty in types {
                match set.piece(ty, false)? {
                    Some(img) => row.push(img),
                    None => return Ok(None),
                }
            }
            let Ok(row) = <[RgbaImage; 3]>::try_from(row) else {
                return Ok(None);
            };
            let (img, fit) = three(row, false)?;
            Ok(Some((img, Some(fit))))
        }
    }
}

/// Fills the surface slot of the component (XML) or section (INI) `name`
/// of the skin file `window`, from its `sets`, when it names one not yet in
/// `controls`; `false` when it names none. `ini` adds the INI era's names.
///
/// # Errors
/// A missing or refused bitmap.
pub fn fill<S: StateImages>(
    (name, window, ini): (&str, Window, bool),
    sets: &[S],
    controls: &mut Vec<Control>,
    tally: &mut StateTally,
) -> Result<bool, ImportError> {
    let extra: &[Surface] = if ini { &INI_SURFACES } else { &[] };
    let Some(found) = SURFACES
        .iter()
        .chain(extra)
        .find(|s| s.window == window && s.name.eq_ignore_ascii_case(name))
        .copied()
    else {
        return Ok(false);
    };
    let (region, pieces) = (found.region, found.pieces);
    if controls.iter().any(|c| c.region == region) {
        return Ok(false);
    }
    let mut images: BTreeMap<RegionState, RgbaImage> = BTreeMap::new();
    let mut fit = button_fit(region, pieces);
    for set in sets {
        let Some(st) = set.state() else { continue };
        let Some((img, row_fit)) = image(set, pieces)? else {
            continue;
        };
        if st != RegionState::Normal && !crate::regions::slot_of(region).is_some_and(|s| s.has(st))
        {
            tally.dropped.push((
                format!("{name} ({region})"),
                state_name(st),
                "the slot has no such state",
            ));
            continue;
        }
        let normal = images.get(&RegionState::Normal).map(RgbaImage::dimensions);
        if st != RegionState::Normal && normal != Some(img.dimensions()) {
            tally.dropped.push((
                format!("{name} ({region})"),
                state_name(st),
                "its size differs from the normal image's",
            ));
            continue;
        }
        if let Some(f) = row_fit {
            fit = f;
        }
        images.insert(st, img);
    }
    if !images.contains_key(&RegionState::Normal) {
        return Ok(false);
    }
    count_sets(tally, sets);
    for st in images.keys().filter(|s| **s != RegionState::Normal) {
        tally.sets.entry(state_name(*st)).or_default().0 += 1;
    }
    let per_image = match pieces {
        Pieces::Row(types) => types.len(),
        Pieces::Button | Pieces::Fill => 1,
    };
    tally.images.0 += (images.len() - 1) * per_image;
    if found.selected_from_down {
        if let Some(down) = images.get(&RegionState::Down).cloned() {
            images.insert(RegionState::Selected, down);
        }
    }
    tally.sources.insert(region, name.to_owned());
    controls.push(Control {
        region,
        fit,
        images,
    });
    Ok(true)
}
