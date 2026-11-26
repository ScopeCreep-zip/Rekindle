//! The dialog-control roles (plan E7.7.3 item 1, E7.7.7; research
//! `research-settings-theme-2026-11-26.md` §2.1, §3.2): the colours Xfire's
//! `Themes.xml` gives its text fields, selection, menu bar, progress bars,
//! scrollbars and tooltips, which a window that draws no pack art
//! (Settings, plan SB1) paints with.
//!
//! This is the one derivation table for them (ST1 ruling 4, 2026-11-26):
//! an import writes a role the pack's theme does not name from these rows
//! ([`fill`], after the base walk), and [`super::resolve`] serves the same
//! rows at load to a package that lacks a role (xfire-classic, imported
//! before the roles existed), so both paths read one table. Every role's
//! contrast pair is a constraint here, so the validator and the import walk
//! hold it to WCAG 1.4.3 (text, 4.5:1) or 1.4.11 (a boundary or a fill,
//! 3:1). A text role is held only against the ground the pack itself draws
//! it on (ST1 ruling 3): `menubar.text` on `menubar.bg`, `progress.text` on
//! the bar, `tooltip.text` on `tooltip.bg`.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::TokenValue;

use super::table::{c1, c2, c4, derived, row, same_as, NONE};
use super::{Base, Constraint, Derivation, TokenEntry};

const fn on(grounds: &'static [&'static str], min_ratio_x100: u16) -> [Constraint; 1] {
    [Constraint {
        on: grounds,
        min_ratio_x100,
    }]
}

/// WCAG 1.4.3: the field text on its field and on an unfocused selection.
const INPUT_TEXT: &[Constraint] = &on(&["input.bg", "input.selection.bg-inactive"], 450);
/// WCAG 1.4.3: selected text on the selected row and the field selection.
const SELECTED_TEXT: &[Constraint] = &on(&["accent.fill", "input.selection.bg"], 450);
/// WCAG 1.4.3: Xfire draws `MenuTextCG` only on its menu bar (ST1 ruling 3).
const MENU_TEXT: &[Constraint] = &on(&["menubar.bg"], 450);
/// WCAG 1.4.11: a slider's fill against its track.
const FILL: &[Constraint] = &on(&["progress.bg"], 300);
/// WCAG 1.4.11: a track's outline against the window ground.
const OUTLINE: &[Constraint] = &on(&["bg.dark"], 300);
/// WCAG 1.4.3: Xfire draws `ProgBarTextCG` on its bar's well (ST1 ruling
/// 3: a text role on the ground the pack drew it on, never the panel).
const PROGRESS_TEXT: &[Constraint] = &on(&["progress.bg"], 450);
/// WCAG 1.4.11: the thumb against its track.
const THUMB: &[Constraint] = &on(&["scrollbar.track"], 300);
const TOOLTIP_TEXT: &[Constraint] = &on(&["tooltip.bg"], 450);

/// The title-bar ground of a window that draws no pack art (Settings,
/// plan SB1). An Xfire import writes the mean colours of the pack's own
/// title strip ends (`import::dialog_roles`); a skin that sets its accent
/// and omits these follows its selection colour toward its ground.
const TITLE_START: TokenEntry = derived(
    row(
        "titlebar.start",
        "--color-xfire-titlebar-start",
        c2("#0f1a3a", "#dbe7f6"),
        NONE,
    ),
    Derivation::Mix {
        a: "accent.fill",
        b: "bg.primary",
        a_weight: 500,
    },
);
const TITLE_END: TokenEntry = derived(
    row(
        "titlebar.end",
        "--color-xfire-titlebar-end",
        c2("#1a2a5e", "#c3d6ef"),
        NONE,
    ),
    Derivation::Mix {
        a: "accent.fill",
        b: "bg.primary",
        a_weight: 250,
    },
);

const fn underived(mut e: TokenEntry) -> TokenEntry {
    e.derivation = None;
    e
}

/// The title grounds as the base walk holds `titlebar.text` to them: their
/// defaults, as before the roles existed (ST1 ruling 2), so a pack's own
/// colours never move for a ground the import derives.
pub const TITLE_AS_BEFORE: [TokenEntry; 2] = [underived(TITLE_START), underived(TITLE_END)];

/// The dialog roles, after the base table (their constraints name rows of
/// both), in derivation order: a row may read an earlier one.
pub const DIALOG: &[TokenEntry] = &[
    TITLE_START,
    TITLE_END,
    same_as(
        row(
            "input.bg",
            "--color-xfire-input-bg",
            c4("#1e1e1e", "#ffffff", "#000000", "#ffffff"),
            NONE,
        ),
        "bg.panel",
    ),
    same_as(
        row(
            "input.text",
            "--color-xfire-input-text",
            c4("#e7e7e7", "#1a1a1a", "#ffffff", "#000000"),
            INPUT_TEXT,
        ),
        "text.base",
    ),
    same_as(
        row(
            "input.selection.bg",
            "--color-xfire-input-selection-bg",
            c1("#167bc0"),
            NONE,
        ),
        "accent.fill",
    ),
    derived(
        row(
            "input.selection.bg-inactive",
            "--color-xfire-input-selection-bg-inactive",
            c4("#3a3a3a", "#d4d4d4", "#404040", "#c0c0c0"),
            NONE,
        ),
        Derivation::Mix {
            a: "accent.fill",
            b: "input.bg",
            a_weight: 400,
        },
    ),
    // Xfire's `ListColorSelectedTextCG`: text on a selected row (and a
    // primary button, the same fill). `accent.fg` stays the fixed white.
    same_as(
        row(
            "selection.text",
            "--color-xfire-selection-text",
            c1("#ffffff"),
            SELECTED_TEXT,
        ),
        "text.base",
    ),
    // The menu bar's ground: an Xfire import writes the alpha-weighted
    // mean of the pack's own menu-bar art (`import::dialog_roles`), as it
    // does the title strip; a skin with neither art nor the colour follows
    // its window ground, where the app lays the bar.
    same_as(
        row(
            "menubar.bg",
            "--color-xfire-menubar-bg",
            c4("#0a0a0a", "#ffffff", "#000000", "#ffffff"),
            NONE,
        ),
        "bg.dark",
    ),
    same_as(
        row(
            "menubar.text",
            "--color-xfire-menubar-text",
            c4("#8fb9d7", "#0d4363", "#ffffff", "#000000"),
            MENU_TEXT,
        ),
        "text.base",
    ),
    same_as(
        row(
            "progress.bg",
            "--color-xfire-progress-bg",
            c4("#1e1e1e", "#ffffff", "#000000", "#ffffff"),
            NONE,
        ),
        "bg.panel",
    ),
    same_as(
        row(
            "progress.fill",
            "--color-xfire-progress-fill",
            c1("#167bc0"),
            FILL,
        ),
        "accent.fill",
    ),
    derived(
        row(
            "progress.outline",
            "--color-xfire-progress-outline",
            c4("#6b6b6b", "#767676", "#ffffff", "#000000"),
            OUTLINE,
        ),
        Derivation::Mix {
            a: "text.base",
            b: "progress.bg",
            a_weight: 500,
        },
    ),
    same_as(
        row(
            "progress.text",
            "--color-xfire-progress-text",
            c4("#e7e7e7", "#1a1a1a", "#ffffff", "#000000"),
            PROGRESS_TEXT,
        ),
        "text.base",
    ),
    same_as(
        row(
            "scrollbar.track",
            "--color-xfire-scrollbar-track",
            c4("#0a0a0a", "#ffffff", "#000000", "#ffffff"),
            NONE,
        ),
        "bg.primary",
    ),
    // No Xfire colour: its thumb is art (`ScrollbarCG` shifts it).
    derived(
        row(
            "scrollbar.thumb",
            "--color-xfire-scrollbar-thumb",
            c4("#7d7d7d", "#737373", "#ffffff", "#000000"),
            THUMB,
        ),
        Derivation::Mix {
            a: "text.base",
            b: "scrollbar.track",
            a_weight: 500,
        },
    ),
    same_as(
        row(
            "tooltip.bg",
            "--color-xfire-tooltip-bg",
            c4("#1a1a1a", "#ffffff", "#000000", "#ffffff"),
            NONE,
        ),
        "bg.secondary",
    ),
    same_as(
        row(
            "tooltip.text",
            "--color-xfire-tooltip-text",
            c4("#e7e7e7", "#1a1a1a", "#ffffff", "#000000"),
            TOOLTIP_TEXT,
        ),
        "text.base",
    ),
];

/// The base row Settings also paints with, which every Xfire import writes
/// too (a contract role; its derivation stays the base table's): the
/// disabled text.
pub const BASE_ROLES: [&str; 1] = ["text.disabled"];

/// Whether `id` is a dialog role: a [`DIALOG`] row or one of
/// [`BASE_ROLES`]. The dialog walk moves only these (ST1 ruling 1).
#[must_use]
pub fn is_role(id: &str) -> bool {
    BASE_ROLES.contains(&id) || DIALOG.iter().any(|e| e.id == id)
}

/// Every dialog role.
pub fn roles() -> impl Iterator<Item = &'static str> {
    BASE_ROLES.into_iter().chain(DIALOG.iter().map(|e| e.id))
}

/// `tokens` with every dialog role it omits set to what
/// [`super::resolve`] serves for it on `base`: the import writes exactly
/// what load would derive, from one table (ST1 ruling 4).
pub fn fill(tokens: &mut BTreeMap<String, TokenValue>, base: Base) {
    let served = super::resolve(super::ENTRIES, base, tokens);
    for (e, v) in served {
        if is_role(e.id) && !tokens.contains_key(e.id) {
            tokens.insert(e.id.to_owned(), v);
        }
    }
}
