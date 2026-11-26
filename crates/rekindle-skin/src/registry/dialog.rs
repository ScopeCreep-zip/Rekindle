//! The dialog-control roles (plan E7.7.3 item 1, E7.7.7; research
//! `research-settings-theme-2026-11-26.md` §2.1, §3.2): the colours Xfire's
//! `Themes.xml` gives its text fields, selection, menu text, progress bars,
//! scrollbars and tooltips, which a window that draws no pack art
//! (Settings, plan SB1) paints with.
//!
//! An Xfire import writes each of them for every pack: from the pack's own
//! colour when it names one (`import::xfire_xml::colours::NAMES`), else
//! from the documented derivation over the colours it does define
//! (`import::dialog_roles::DERIVE`). The derivations here serve a skin of
//! any other format that omits a role; like every registry derivation they
//! read only rows with none. Every role's contrast pair is a constraint
//! here, so the validator and the import walk hold it to WCAG 1.4.3 (text,
//! 4.5:1) or 1.4.11 (a boundary or a fill, 3:1).

use super::table::{c1, c4, derived, row, same_as, NONE};
use super::{Constraint, Derivation, TokenEntry};

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
/// WCAG 1.4.3: text on the window ground (Settings' tab rail).
const WINDOW_TEXT: &[Constraint] = &on(&["bg.dark", "bg.primary"], 450);
/// WCAG 1.4.11: a slider's fill against its track.
const FILL: &[Constraint] = &on(&["progress.bg"], 300);
/// WCAG 1.4.11: a track's outline against the window ground.
const OUTLINE: &[Constraint] = &on(&["bg.dark"], 300);
/// WCAG 1.4.3: a progress value is drawn beside its bar on the panel
/// ground, never on the bar (Settings; lead ruling 2026-11-26).
const PROGRESS_TEXT: &[Constraint] = &on(&["bg.panel"], 450);
/// WCAG 1.4.11: the thumb against its track.
const THUMB: &[Constraint] = &on(&["scrollbar.track"], 300);
const TOOLTIP_TEXT: &[Constraint] = &on(&["tooltip.bg"], 450);

/// The dialog roles, after the base table (their constraints name rows of
/// both).
pub const DIALOG: &[TokenEntry] = &[
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
        "text.primary",
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
            b: "bg.panel",
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
        "text.primary",
    ),
    same_as(
        row(
            "menubar.text",
            "--color-xfire-menubar-text",
            c4("#8fb9d7", "#0d4363", "#ffffff", "#000000"),
            WINDOW_TEXT,
        ),
        "text.primary",
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
            a: "text.primary",
            b: "bg.panel",
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
        "text.primary",
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
            a: "text.primary",
            b: "bg.primary",
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
        "text.primary",
    ),
];
