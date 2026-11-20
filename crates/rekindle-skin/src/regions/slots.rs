//! The region slots (plan E7.7.1 "Regions"): each an app-owned box a skin
//! may paint a bitmap into, with the states it has. `src/styles/
//! regions.toml`'s `[regions]` table maps a slot to the app's elements.
//!
//! * `css` slots are leaf controls that carry no text (titlebar buttons,
//!   scrollbar parts): the app's generated `src/styles/generated/
//!   regions.css` paints them from the skin's custom properties
//!   ([`super::css`]). The frame, titlebar and status bar carry text or
//!   hold the whole window: their selectors and the `textBacking` layer
//!   are E7.7.2's apply step, so they are measured (F83) but not painted.
//! * A painted slot that carries text (`button.secondary`, on
//!   `.form-btn-secondary`) needs its `textBacking` between the bitmap and
//!   the label. Its own `background` cannot be it: `border-image` paints
//!   over the background. Decided (E7.7.2, CSS 2.1 Appendix E painting
//!   order: a stacking context paints its background and borders, then
//!   children with negative `z-index`, then in-flow content) and built
//!   (E7.7.2b): the element gets `isolation: isolate` and a `::before` with
//!   `position: absolute; inset: 0; z-index: -1` whose background is the
//!   backing token, so the stack is image, backing, label with no markup
//!   change ([`super::css`]). The backing covers the image's centre (the
//!   box inset by the nine-slice widths, so the edge slices stay visible)
//!   and the label's content box, so the label is always over it even
//!   where a variant's padding is narrower than a slice. Its F83 box is the
//!   button's own ([`Extent::Range`]), never the window's, and the band it
//!   samples on each side is the smaller of the slice and
//!   [`RegionSlot::label_inset`].
//! * A container slot that holds text in its children (a bar, a header, a
//!   selected row: plan E7.7.7) is `labelled`: `regions.toml` names the
//!   children that carry its text, and each paints the backing as its own
//!   `background-image` over the container's bitmap (CSS Backgrounds 3:
//!   a box's background paints under its content, and a child paints over
//!   its ancestor's border image), so the bitmap shows around the labels
//!   and never under their text. Its F83 box is the container's.
//! * A text slot's backing is one of its `grounds`: the ground tokens its
//!   text is declared on in `regions.toml` (`on = [...]`, plan E7.7.0), so
//!   the pair checks that already hold for that text on that ground keep
//!   holding over any bitmap.
//! * A `sized` slot has no box until a skin fills it: the generator sizes
//!   its element to the image (the status arrow), so without a skin it
//!   takes no room.
//! * A state is one of Rekindle's own enums. A skin image for a state the
//!   slot does not list is refused, so every image a skin ships is one a
//!   selector can show.

use rekindle_types::dto::skin::RegionState;

use crate::limits::MAX_IMAGE_SIDE;

mod build;
use build::{normal, slot};

/// One app window's sizes, CSS px: minimum and maximum inner size.
#[derive(Debug, Clone, Copy)]
pub struct WindowSize {
    pub kind: &'static str,
    pub min: (u32, u32),
    pub max: (u32, u32),
}

const UNBOUNDED: (u32, u32) = (MAX_IMAGE_SIDE, MAX_IMAGE_SIDE);

/// Every window (`src-tauri/src/windows/mod.rs:54-72`, `:107`).
pub const WINDOWS: [WindowSize; 7] = [
    window("login", (340, 440), UNBOUNDED),
    window("buddy-list", (300, 500), (400, 900)),
    window("settings", (420, 450), UNBOUNDED),
    window("chat", (380, 400), UNBOUNDED),
    window("community", (750, 500), UNBOUNDED),
    window("call", (360, 420), UNBOUNDED),
    window("profile", (340, 420), UNBOUNDED),
];

const fn window(kind: &'static str, min: (u32, u32), max: (u32, u32)) -> WindowSize {
    WindowSize { kind, min, max }
}

/// A box side: the window's, a dimension token's value, or a control's
/// own range in CSS px (`min` measured at a window's minimum size, `max`
/// at its maximum; either is capped by the window's side).
#[derive(Debug, Clone, Copy)]
pub enum Extent {
    Window,
    Token(&'static str),
    Range { min: u32, max: u32 },
}

/// A text role drawn on a region.
#[derive(Debug, Clone, Copy)]
pub struct RegionText {
    /// The registry colour token of the text.
    pub token: &'static str,
    /// WCAG large text (≥ 18 pt, or ≥ 14 pt bold): 3:1 instead of 4.5:1.
    pub large: bool,
}

/// A region slot.
#[derive(Debug, Clone, Copy)]
pub struct RegionSlot {
    pub id: &'static str,
    pub width: Extent,
    pub height: Extent,
    /// The text drawn on it; empty when it carries none.
    pub text: &'static [RegionText],
    /// The states a skin may give an image for, besides normal.
    pub states: &'static [RegionState],
    /// Painted by the app's generated `regions.css`.
    pub css: bool,
    /// A painted text slot's least padding (horizontal, vertical) between
    /// its padding edge and its label over all its variants: the backing
    /// reaches at least this far in. Zero for a slot backed edge to edge.
    pub label_inset: (u32, u32),
    /// The registry colour tokens its text is declared on: a skin's
    /// `textBacking` must be one of them. Empty when it carries no text.
    pub grounds: &'static [&'static str],
    /// Its text is on children `regions.toml` names (`labels`), each
    /// backed in full; the element itself carries none.
    pub labelled: bool,
    /// The generator sizes its element to the image (0 without a skin).
    pub sized: bool,
}

use RegionState::{Disabled, Down, Focused, Hover, Selected, WindowInactive};

/// A push button's states (Xfire `PushButton`: Normal, Hover, Down,
/// Disabled; Winamp `_DEPRESSED`).
const BUTTON: &[RegionState] = &[Hover, Down, Disabled, Focused, WindowInactive];
/// A scrollbar part's (WebKit's scrollbar pseudo-classes).
const SCROLL: &[RegionState] = &[Hover, Down, Disabled, WindowInactive];
const CHROME: Extent = Extent::Token("size.chrome-height");
/// `.form-btn-secondary`'s width: its narrowest is a one-glyph label in
/// `.settings-member-actions` (`07-member-profile-popup.css:58`, 6 px
/// padding each side, 1 px borders, a 10 px glyph); its widest fills the
/// window.
const SECONDARY_WIDTH: Extent = Extent::Range {
    min: 20,
    max: MAX_IMAGE_SIDE,
};
/// Its height: 2 px padding and a 10 px font (≈ 12 px line) with 1 px
/// borders at its smallest (`07-member-profile-popup.css:58`); 8 px
/// padding, a 12 px font and the 2 px borders of `prefers-contrast: more`
/// (`00-rules.css:207`, `91-top-level.css:368`) at its largest, rounded
/// up to 40.
const SECONDARY_HEIGHT: Extent = Extent::Range { min: 16, max: 40 };
/// Its least padding: 6 px each side and 2 px top and bottom, in
/// `.settings-member-actions` (`07-member-profile-popup.css:58`; the others
/// are 16/8 px at `00-rules.css:207` and 10/5 px at `90-top-level.css:31`).
const SECONDARY_LABEL_INSET: (u32, u32) = (6, 2);
/// `.form-btn-primary`'s least padding: 8 px each side and 4 px top and
/// bottom (`10-thread-list-panel.css:344`; 8 px at `00-rules.css:214`,
/// 6/12 px at `:305`).
const PRIMARY_LABEL_INSET: (u32, u32) = (8, 4);
/// A buddy-list tab's states: Xfire's tab buttons are `PushType="Sticky"`
/// in one `GroupId` (Symbiosis `Components/MainWindow.xml:910`), so the
/// chosen tab keeps its Down image: [`Selected`].
const TAB: &[RegionState] = &[Hover, Down, Selected, Focused, WindowInactive];
/// `.buddy-tab` (`08-community-events.css:131`): `flex: 1` of a window
/// 300–400 px wide, two tabs, so 150–200 px; measured from 40 for a
/// third tab. 6 px padding around an 11 px label (≈ 25 px) up to the 2 px
/// borders of `prefers-contrast: more`.
const TAB_WIDTH: Extent = Extent::Range { min: 40, max: 400 };
const TAB_HEIGHT: Extent = Extent::Range { min: 24, max: 40 };
/// Its padding: 8 px each side, 6 px top and bottom.
const TAB_LABEL_INSET: (u32, u32) = (8, 6);
/// `.action-bar-icon-btn` (`06-member-profile-popup.css:310`): 4/8 px
/// padding around a 14 px glyph.
const ACTION_WIDTH: Extent = Extent::Range { min: 24, max: 64 };
const ACTION_HEIGHT: Extent = Extent::Range { min: 20, max: 40 };
const ACTION_LABEL_INSET: (u32, u32) = (8, 4);
/// The buddy-list window's ground (`.app-frame`, `regions.toml`
/// `'.buddy-tab' = { on = ["--color-xfire-bg-dark"] }`).
const ON_DARK: &[&str] = &["bg.dark"];
/// The chat toolbar's ground (`.chat-peer-status`: `bg.panel` at 80% over
/// the window's `bg.dark`); a skinned bar backs its labels with `bg.panel`.
const ON_PANEL: &[&str] = &["bg.panel"];
/// The Send button: a four-letter 11 px label (≈ 26 px) in 10 px padding
/// and 1 px borders, up to the 2 px borders of `prefers-contrast: more`.
const SEND_WIDTH: Extent = Extent::Range { min: 40, max: 96 };
const SEND_LABEL_INSET: (u32, u32) = (10, 2);
/// The chat window's call buttons hold one 14 px glyph in 24 px.
const CALL_LABEL_INSET: (u32, u32) = (4, 4);

/// The region slots. Every slot appears in every window.
pub const SLOTS: [RegionSlot; 36] = [
    // The whole window behind its content. DoiDD's frame has hover, down
    // and disabled (window-inactive) images.
    slot("window.frame", Extent::Window, Extent::Window, &[]).states(&[
        Hover,
        Down,
        WindowInactive,
    ]),
    // `.xfire-titlebar` (`00-rules.css:98`), its title `.xfire-titlebar-text`
    // (`00-primitives.css:102`, 11 px semibold).
    // Its labels are the title and the titlebar buttons (their glyphs are
    // `titlebar.text` at 60%): each backed in full, so the title keeps a
    // solid ground over a skin's titlebar strip (Classic v3
    // `MainTitleBar`, its Disabled set drawn while the window is inactive).
    slot(
        "titlebar",
        Extent::Window,
        CHROME,
        &[normal("titlebar.text")],
    )
    .labelled(&[WindowInactive], &["titlebar.start", "titlebar.end"]),
    // Every titlebar button (Miranda's one `Titlebutton` set).
    slot("titlebar.buttons", CHROME, CHROME, &[]).states(BUTTON),
    // One titlebar button each (Xfire `ButtonMinimize` …; Winamp
    // TITLEBAR.BMP). The glyph is drawn over the image, never replaced.
    slot("titlebar.btn.minimize", CHROME, CHROME, &[]).painted(BUTTON),
    slot("titlebar.btn.maximize", CHROME, CHROME, &[]).painted(BUTTON),
    slot("titlebar.btn.close", CHROME, CHROME, &[]).painted(BUTTON),
    // The buddy list's `.status-bar` (`01-form-primitives.css:82`, 11 px,
    // on `bg.status`): its labels are the status picker (`.buddy-name`,
    // `text.base`, beside a status dot) and the network indicator
    // (`text.dim`). Xfire `StatusBarContainer`.
    slot(
        "statusbar",
        Extent::Window,
        CHROME,
        &[
            normal("text.status"),
            normal("text.base"),
            normal("text.dim"),
        ],
    )
    .labelled(&[], &["bg.status"]),
    // The status picker's arrow (Xfire `ButtonStatusChanger`, 21 × 19, an
    // arrow glyph and no label), sized to the image.
    slot("statusbar.arrow", Extent::Window, Extent::Window, &[])
        .painted(&[Hover, Down])
        .sized(),
    // `.menu-bar` (`08-community-events.css:170`, 22 px; its items
    // `text.dim`, `text.base` on hover). Xfire `MenuBackground`.
    slot(
        "buddylist.menubar",
        Extent::Window,
        Extent::Range { min: 22, max: 22 },
        &[normal("text.dim"), normal("text.base")],
    )
    .labelled(&[], &["bg.dark", "bg.panel"]),
    // `.identity-bar` (`04-member-profile-popup.css:281`): the name
    // (`status.offline-text`), key (`text.dim`, `link` on hover), game
    // (`accent.text`) and status message (`text.dim`). Xfire `InfoBar`.
    slot(
        "buddylist.infobar",
        Extent::Window,
        Extent::Range { min: 36, max: 64 },
        &[
            normal("status.offline-text"),
            normal("text.dim"),
            normal("link"),
            normal("accent.text"),
        ],
    )
    .labelled(&[], ON_DARK),
    // `.buddy-tab` (`08-community-events.css:131`): `text.dim`,
    // `text.base` on hover, `accent.text` when active. Xfire's tab
    // buttons (`ButtonFriendsTab` …).
    slot(
        "buddylist.tab",
        TAB_WIDTH,
        TAB_HEIGHT,
        &[
            normal("text.dim"),
            normal("text.base"),
            normal("accent.text"),
        ],
    )
    .painted(TAB)
    .label_inset(TAB_LABEL_INSET)
    .grounds(ON_DARK),
    // `.buddy-group-header` (`00-rules.css:363`, `text.dim`, its arrow,
    // name and count spans). Xfire's list header (`XfireView`
    // `HeaderLeft/Middle/Right`).
    slot(
        "buddylist.group-header",
        Extent::Window,
        Extent::Range { min: 20, max: 32 },
        &[normal("text.dim")],
    )
    // Clicking the header folds the group, so it shows a hover image
    // (Clist_modern `$$Group/Open/Hot`); its text stays `text.dim`.
    .labelled(&[Hover], ON_DARK),
    // `.buddy-item-selected` (`00-rules.css:392`): the row's content
    // (name, status line, game). Xfire paints it with a colour
    // (`ListColorSelectedCG`), so no Xfire image fills it.
    slot(
        "buddylist.row-selected",
        Extent::Window,
        Extent::Range { min: 20, max: 64 },
        &[
            normal("text.primary"),
            normal("text.secondary"),
            normal("status.offline-text"),
            normal("accent.text"),
        ],
    )
    .labelled(&[], ON_DARK),
    // `.action-bar` (`05-member-profile-popup.css:127`): its buttons'
    // glyphs (`accent.text`; logout `text.dim`). Xfire `FriendsButtonBar`.
    slot(
        "buddylist.actionbar",
        Extent::Window,
        Extent::Range { min: 24, max: 40 },
        &[normal("accent.text"), normal("text.dim")],
    )
    .labelled(&[], ON_DARK),
    // `.action-bar-icon-btn` (`06-member-profile-popup.css:310`, glyph
    // `accent.text`): the frame of each action button; the glyph is drawn
    // over the backing. Xfire's friends buttons (`ButtonAddFriend`,
    // `ButtonInfo`) bake their label into the bitmap, which the backing
    // covers: only their bevel shows.
    slot(
        "buddylist.action-button",
        ACTION_WIDTH,
        ACTION_HEIGHT,
        &[normal("accent.text")],
    )
    .painted(BUTTON)
    .label_inset(ACTION_LABEL_INSET)
    .grounds(ON_DARK),
    slot(
        "chat.toolbar",
        Extent::Window,
        CHROME,
        &[
            normal("text.dim"),
            normal("status.online-text"),
            normal("danger.text"),
        ],
    )
    .labelled(&[], ON_PANEL),
    slot(
        "chat.call-button",
        ACTION_WIDTH,
        ACTION_HEIGHT,
        &[normal("status.online-text")],
    )
    .painted(BUTTON)
    .label_inset(CALL_LABEL_INSET)
    .grounds(ON_PANEL),
    slot(
        "chat.call-button-active",
        ACTION_WIDTH,
        ACTION_HEIGHT,
        &[normal("danger.text")],
    )
    .painted(BUTTON)
    .label_inset(CALL_LABEL_INSET)
    .grounds(ON_PANEL),
    slot(
        "chat.input",
        Extent::Window,
        Extent::Range { min: 34, max: 96 },
        &[
            normal("text.dim"),
            normal("accent.text"),
            normal("danger.text"),
        ],
    )
    .labelled(&[], ON_DARK),
    // `.message-input-send-btn` (`13-bulk-message-delete-mod-selection-.css`,
    // 11 px semibold `text.base`, `text.dim` disabled, 2/10 px padding) at
    // the input's end, on its ground: Xfire's Send button (Symbiosis
    // `ChatButtonSend`, Classic v3 `SendButton`, Combat `[ImButton]`).
    slot(
        "chat.send-button",
        SEND_WIDTH,
        ACTION_HEIGHT,
        &[normal("text.base"), normal("text.dim")],
    )
    .painted(BUTTON)
    .label_inset(SEND_LABEL_INSET)
    .grounds(ON_DARK),
    slot(
        "chat.group-header",
        Extent::Window,
        Extent::Range { min: 20, max: 40 },
        &[normal("text.dim")],
    )
    .labelled(&[], ON_DARK),
    // Plan E7.7.7 S5: the community window (`.community-window`,
    // CommunityWindow.tsx), Xfire's chat-room window. Its sidebar headers
    // (`.community-sidebar-header`, `02-form-primitives.css:46`, `text.dim`
    // title, `accent.text` add buttons, on `bg.dark`): Xfire's info bar.
    slot(
        "community.sidebar-header",
        Extent::Window,
        Extent::Range { min: 24, max: 48 },
        &[normal("text.dim"), normal("accent.text")],
    )
    .labelled(&[], ON_DARK),
    // A channel category (`.category-header`, `text.dim`): folding it is a
    // click, so it has a hover image as the buddy list's group header has.
    // Xfire's list header (`XfireView` `Header*`).
    slot(
        "community.category-header",
        Extent::Window,
        Extent::Range { min: 20, max: 40 },
        &[normal("text.dim")],
    )
    .labelled(&[Hover], ON_DARK),
    // The channel header bar (`.community-channel-header`,
    // `02-form-primitives.css:395`, `bg.dark`): icon, name, topic and
    // badges `text.dim`, panel buttons `accent.text`. Xfire's chat button
    // bar (`ChatButtonBar`).
    slot(
        "community.channel-header",
        Extent::Window,
        Extent::Range { min: 28, max: 56 },
        &[normal("text.dim"), normal("accent.text")],
    )
    .labelled(&[], ON_DARK),
    // A member list role header (`.member-group-header`,
    // `00-primitives.css:172`, `text.dim` on `bg.dark`): Xfire's list
    // header, as the category header.
    slot(
        "community.member-group-header",
        Extent::Window,
        Extent::Range { min: 20, max: 40 },
        &[normal("text.dim")],
    )
    .labelled(&[], ON_DARK),
    // `::-webkit-scrollbar-*` (`scrollbar.css`); Xfire `XfireScrollbar`,
    // Winamp GENEX.
    slot("scrollbar.track", Extent::Window, Extent::Window, &[]).painted(SCROLL),
    slot("scrollbar.thumb", Extent::Window, Extent::Window, &[]).painted(SCROLL),
    slot(
        "scrollbar.track-horizontal",
        Extent::Window,
        Extent::Window,
        &[],
    )
    .painted(SCROLL),
    slot(
        "scrollbar.thumb-horizontal",
        Extent::Window,
        Extent::Window,
        &[],
    )
    .painted(SCROLL),
    slot("scrollbar.button-up", Extent::Window, Extent::Window, &[]).painted(SCROLL),
    slot("scrollbar.button-down", Extent::Window, Extent::Window, &[]).painted(SCROLL),
    slot("scrollbar.button-left", Extent::Window, Extent::Window, &[]).painted(SCROLL),
    slot(
        "scrollbar.button-right",
        Extent::Window,
        Extent::Window,
        &[],
    )
    .painted(SCROLL),
    slot("scrollbar.corner", Extent::Window, Extent::Window, &[]).painted(&[WindowInactive]),
    // `.form-btn-secondary` (`00-rules.css:207`, `--color-xfire-text`, 12
    // px semibold); Winamp GENEX's button. The label is drawn over the
    // backing, which is drawn over the image.
    slot(
        "button.secondary",
        SECONDARY_WIDTH,
        SECONDARY_HEIGHT,
        &[normal("text.base")],
    )
    .painted(BUTTON)
    .label_inset(SECONDARY_LABEL_INSET)
    .grounds(&["bg.panel", "bg.dark", "bg.secondary"]),
    // `.form-btn-primary` (`00-rules.css:212`, `accent.fg` 12 px
    // semibold on `accent.fill`): a dialog's confirming button; Xfire's OK
    // (Symbiosis `button_ok_*`).
    slot(
        "button.primary",
        SECONDARY_WIDTH,
        SECONDARY_HEIGHT,
        &[normal("accent.fg")],
    )
    .painted(BUTTON)
    .label_inset(PRIMARY_LABEL_INSET)
    .grounds(&["accent.fill"]),
];

/// The slot `id`.
#[must_use]
pub fn slot_of(id: &str) -> Option<&'static RegionSlot> {
    SLOTS.iter().find(|s| s.id == id)
}
