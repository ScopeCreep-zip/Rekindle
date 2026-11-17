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
//!   change ([`super::css`]). Its F83 box is the button's own
//!   ([`Extent::Range`]), never the window's.
//! * A state is one of Rekindle's own enums. A skin image for a state the
//!   slot does not list is refused, so every image a skin ships is one a
//!   selector can show.

use rekindle_types::dto::skin::RegionState;

use crate::limits::MAX_IMAGE_SIDE;

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
}

use RegionState::{Disabled, Down, Focused, Hover, WindowInactive};

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

/// The region slots. Every slot appears in every window.
pub const SLOTS: [RegionSlot; 17] = [
    // The whole window behind its content. DoiDD's frame has hover, down
    // and disabled (window-inactive) images.
    slot("window.frame", Extent::Window, Extent::Window, &[]).states(&[
        Hover,
        Down,
        WindowInactive,
    ]),
    // `.xfire-titlebar` (`00-rules.css:98`), its title `.xfire-titlebar-text`
    // (`00-primitives.css:102`, 11 px semibold).
    slot(
        "titlebar",
        Extent::Window,
        CHROME,
        &[normal("titlebar.text")],
    )
    .states(&[WindowInactive]),
    // Every titlebar button (Miranda's one `Titlebutton` set).
    slot("titlebar.buttons", CHROME, CHROME, &[]).states(BUTTON),
    // `.status-bar` (`01-form-primitives.css:62`, 11 px).
    slot(
        "statusbar",
        Extent::Window,
        CHROME,
        &[normal("text.status")],
    ),
    // One titlebar button each (Xfire `ButtonMinimize` …; Winamp
    // TITLEBAR.BMP). The glyph is drawn over the image, never replaced.
    slot("titlebar.btn.minimize", CHROME, CHROME, &[]).painted(BUTTON),
    slot("titlebar.btn.maximize", CHROME, CHROME, &[]).painted(BUTTON),
    slot("titlebar.btn.close", CHROME, CHROME, &[]).painted(BUTTON),
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
    .painted(BUTTON),
];

const fn normal(token: &'static str) -> RegionText {
    RegionText {
        token,
        large: false,
    }
}

const fn slot(
    id: &'static str,
    width: Extent,
    height: Extent,
    text: &'static [RegionText],
) -> RegionSlot {
    RegionSlot {
        id,
        width,
        height,
        text,
        states: &[],
        css: false,
    }
}

impl RegionSlot {
    const fn states(self, states: &'static [RegionState]) -> Self {
        Self { states, ..self }
    }

    const fn painted(self, states: &'static [RegionState]) -> Self {
        Self {
            states,
            css: true,
            ..self
        }
    }

    /// Whether the slot has `state` (normal is the slot's image).
    #[must_use]
    pub fn has(&self, state: RegionState) -> bool {
        self.states.contains(&state)
    }
}

/// The slot `id`.
#[must_use]
pub fn slot_of(id: &str) -> Option<&'static RegionSlot> {
    SLOTS.iter().find(|s| s.id == id)
}
