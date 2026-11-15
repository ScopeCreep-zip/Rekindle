//! The registry table, seeded from the hand-written token blocks of
//! `src/styles/global.css` it replaces (plan E7.7.1).
//!
//! Dark values are the old `@theme` block, unchanged. Light values are the
//! old `@layer xfire-defaults` block plus, per the plan's "Light defaults",
//! grounds of its own (white, the ground every light value was calibrated
//! on, replacing `check-contrast-pairs`' stand-in) and a light value for
//! every text role, each reaching 4.5:1 on every declared light ground
//! (`cargo xtask check-contrast-pairs`, light pass). High-contrast defaults
//! put text at black or white on white or black grounds; the status,
//! mention and fill colours keep their standard values. These are not
//! emitted to the app's CSS (the `prefers-contrast` overrides live in
//! `@layer a11y`); they are the defaults a high-contrast skin variant
//! resolves against.

use super::{Class, Constraint, DefaultValue, Defaults, Derivation, Entry, TokenType};

const fn digit(b: u8) -> u32 {
    match b {
        b'0'..=b'9' => (b - b'0') as u32,
        b'a'..=b'f' => (b - b'a' + 10) as u32,
        _ => panic!("a registry colour is #rrggbb, lowercase"),
    }
}

/// `#rrggbb`, checked when the table is compiled.
const fn rgb(hex: &str) -> DefaultValue {
    let b = hex.as_bytes();
    assert!(b.len() == 7 && b[0] == b'#', "a registry colour is #rrggbb");
    let mut v = 0u32;
    let mut i = 1;
    while i < 7 {
        v = v * 16 + digit(b[i]);
        i += 1;
    }
    DefaultValue::Color((v << 8) | 0xff)
}

/// Dark, light, high-contrast dark, high-contrast light.
const fn c4(dark: &str, light: &str, hc_dark: &str, hc_light: &str) -> Defaults {
    Defaults {
        dark: rgb(dark),
        light: rgb(light),
        hc_dark: rgb(hc_dark),
        hc_light: rgb(hc_light),
    }
}

/// High contrast keeps the standard value of each base.
const fn c2(dark: &str, light: &str) -> Defaults {
    c4(dark, light, dark, light)
}

const fn c1(all: &str) -> Defaults {
    c4(all, all, all, all)
}

const fn font_all(name: &'static str) -> Defaults {
    let f = DefaultValue::Font(name);
    Defaults {
        dark: f,
        light: f,
        hc_dark: f,
        hc_light: f,
    }
}

const fn row(
    id: &'static str,
    css: &'static str,
    defaults: Defaults,
    constraints: &'static [Constraint],
) -> Entry {
    Entry {
        id,
        css,
        ty: TokenType::Color,
        defaults,
        derivation: None,
        constraints,
        class: Class::Themeable,
    }
}

const fn fixed(mut e: Entry) -> Entry {
    e.class = Class::Fixed;
    e
}

/// Follows `of` when a skin sets `of` and omits this token.
const fn same_as(mut e: Entry, of: &'static str) -> Entry {
    e.derivation = Some(Derivation::Mix {
        a: of,
        b: of,
        a_weight: 1000,
    });
    e
}

/// The opaque grounds text is laid on.
const GROUNDS: &[&str] = &[
    "bg.dark",
    "bg.base",
    "bg.primary",
    "bg.secondary",
    "bg.panel",
    "bg.tertiary",
    "bg.input",
    "bg.status",
];
/// WCAG 1.4.3: text 4.5:1 on every ground.
const TEXT: &[Constraint] = &[Constraint {
    on: GROUNDS,
    min_ratio_x100: 450,
}];
/// WCAG 1.4.11: status graphics and boundaries 3:1 on the window and panel.
const GRAPHIC: &[Constraint] = &[Constraint {
    on: &["bg.dark", "bg.panel"],
    min_ratio_x100: 300,
}];
const NONE: &[Constraint] = &[];

pub const ENTRIES: &[Entry] = &[
    row(
        "bg.dark",
        "--color-xfire-bg-dark",
        c4("#0a0a0a", "#ffffff", "#000000", "#ffffff"),
        NONE,
    ),
    row(
        "bg.panel",
        "--color-xfire-bg-panel",
        c4("#1e1e1e", "#ffffff", "#000000", "#ffffff"),
        NONE,
    ),
    same_as(
        row(
            "bg.input",
            "--color-xfire-bg-input",
            c4("#1e1e1e", "#ffffff", "#000000", "#ffffff"),
            NONE,
        ),
        "bg.panel",
    ),
    row(
        "bg.status",
        "--color-xfire-bg-status",
        c4("#181818", "#ffffff", "#000000", "#ffffff"),
        NONE,
    ),
    same_as(
        row(
            "bg.base",
            "--color-xfire-bg",
            c4("#0a0a0a", "#ffffff", "#000000", "#ffffff"),
            NONE,
        ),
        "bg.dark",
    ),
    row(
        "bg.primary",
        "--color-xfire-bg-primary",
        c4("#141414", "#ffffff", "#000000", "#ffffff"),
        NONE,
    ),
    row(
        "bg.secondary",
        "--color-xfire-bg-secondary",
        c4("#1a1a1a", "#ffffff", "#000000", "#ffffff"),
        NONE,
    ),
    row(
        "bg.tertiary",
        "--color-xfire-bg-tertiary",
        c4("#252525", "#ffffff", "#000000", "#ffffff"),
        NONE,
    ),
    row(
        "border",
        "--color-xfire-border",
        c4("#2a2a2a", "#d4d4d4", "#ffffff", "#000000"),
        NONE,
    ),
    row(
        "control-border",
        "--color-xfire-control-border",
        c4("#6b6b6b", "#767676", "#ffffff", "#000000"),
        GRAPHIC,
    ),
    row(
        "text.primary",
        "--color-xfire-text-primary",
        c4("#e7e7e7", "#1a1a1a", "#ffffff", "#000000"),
        TEXT,
    ),
    row(
        "text.secondary",
        "--color-xfire-text-secondary",
        c4("#a0a0a0", "#595959", "#ffffff", "#000000"),
        TEXT,
    ),
    row("accent.fill", "--color-xfire-accent", c1("#167bc0"), NONE),
    row(
        "accent.text",
        "--color-xfire-accent-text",
        c4("#3491d8", "#0a5a9c", "#8cc4ff", "#0b4f8a"),
        TEXT,
    ),
    fixed(row(
        "accent.fg",
        "--color-xfire-accent-fg",
        c1("#ffffff"),
        &[Constraint {
            on: &["accent.fill"],
            min_ratio_x100: 450,
        }],
    )),
    row("danger.fill", "--color-xfire-danger", c1("#df3337"), NONE),
    fixed(row(
        "danger.fg",
        "--color-xfire-danger-fg",
        c1("#ffffff"),
        &[Constraint {
            on: &["danger.fill"],
            min_ratio_x100: 450,
        }],
    )),
    row(
        "danger.text",
        "--color-xfire-danger-text",
        c4("#ff5955", "#b4001a", "#ff8a80", "#8a0000"),
        TEXT,
    ),
    fixed(row(
        "text.on-fill",
        "--color-xfire-text-on-fill",
        c1("#ffffff"),
        &[Constraint {
            on: &["accent.fill", "danger.fill", "success.fill"],
            min_ratio_x100: 450,
        }],
    )),
    same_as(
        row(
            "text.base",
            "--color-xfire-text",
            c4("#e7e7e7", "#1a1a1a", "#ffffff", "#000000"),
            TEXT,
        ),
        "text.primary",
    ),
    row(
        "text.dim",
        "--color-xfire-text-dim",
        c4("#8fb9d7", "#2d5f80", "#ffffff", "#000000"),
        TEXT,
    ),
    row(
        "text.clan",
        "--color-xfire-text-clan",
        c4("#1ba1fd", "#0a5a9c", "#8cc4ff", "#0b4f8a"),
        TEXT,
    ),
    row(
        "text.status",
        "--color-xfire-text-status",
        c4("#969696", "#595959", "#ffffff", "#000000"),
        TEXT,
    ),
    same_as(
        row(
            "text.timestamp",
            "--color-xfire-text-timestamp",
            c4("#a0a0a0", "#595959", "#ffffff", "#000000"),
            TEXT,
        ),
        "text.secondary",
    ),
    row(
        "status.online",
        "--color-xfire-online",
        c2("#53d769", "#008a2e"),
        GRAPHIC,
    ),
    row(
        "status.away",
        "--color-xfire-away",
        c2("#ffcc00", "#927300"),
        GRAPHIC,
    ),
    row(
        "status.ingame",
        "--color-xfire-ingame",
        c2("#4fc3f7", "#0080ab"),
        GRAPHIC,
    ),
    row(
        "status.online-text",
        "--color-xfire-online-text",
        c2("#53d769", "#007427"),
        TEXT,
    ),
    row(
        "status.away-text",
        "--color-xfire-away-text",
        c2("#ffcc00", "#6e5600"),
        TEXT,
    ),
    row(
        "status.ingame-text",
        "--color-xfire-ingame-text",
        c2("#4fc3f7", "#006285"),
        TEXT,
    ),
    row(
        "status.busy",
        "--color-xfire-busy",
        c2("#ef4444", "#df3337"),
        GRAPHIC,
    ),
    row(
        "status.offline",
        "--color-xfire-offline",
        c2("#7d7d7d", "#767676"),
        GRAPHIC,
    ),
    row(
        "status.offline-text",
        "--color-xfire-offline-text",
        c2("#909090", "#5b5b5b"),
        TEXT,
    ),
    row(
        "link",
        "--color-xfire-link",
        c4("#5fa8d3", "#0a5a9c", "#8cc4ff", "#0b4f8a"),
        TEXT,
    ),
    same_as(
        row(
            "focus-outline",
            "--color-xfire-focus-outline",
            c4("#5fa8d3", "#0b5cad", "#ffffff", "#000000"),
            GRAPHIC,
        ),
        "link",
    ),
    row(
        "success.text",
        "--color-xfire-success",
        c2("#51e385", "#006b25"),
        TEXT,
    ),
    row(
        "success.fill",
        "--color-xfire-success-fill",
        c1("#00892d"),
        NONE,
    ),
    row(
        "titlebar.start",
        "--color-xfire-titlebar-start",
        c1("#0f1a3a"),
        NONE,
    ),
    row(
        "titlebar.end",
        "--color-xfire-titlebar-end",
        c1("#1a2a5e"),
        NONE,
    ),
    fixed(row("shadow", "--color-xfire-shadow", c1("#000000"), NONE)),
    fixed(row(
        "scrim.fill",
        "--color-xfire-scrim",
        c1("#000000"),
        NONE,
    )),
    fixed(row(
        "scrim.danger-text",
        "--color-xfire-scrim-danger-text",
        c1("#ff5955"),
        &[Constraint {
            on: &["scrim.fill"],
            min_ratio_x100: 450,
        }],
    )),
    fixed(row(
        "video-backing",
        "--color-xfire-video-backing",
        c1("#000000"),
        NONE,
    )),
    row(
        "overlay",
        "--color-xfire-overlay",
        c4("#ffffff", "#000000", "#ffffff", "#000000"),
        NONE,
    ),
    row(
        "shade",
        "--color-xfire-shade",
        c4("#000000", "#ffffff", "#000000", "#ffffff"),
        NONE,
    ),
    fixed(row(
        "fill-shade",
        "--color-xfire-fill-shade",
        c1("#000000"),
        NONE,
    )),
    row(
        "thread-tag",
        "--color-xfire-thread-tag",
        c2("#ffd479", "#7a5500"),
        TEXT,
    ),
    row(
        "mention.everyone",
        "--color-xfire-mention-everyone",
        c2("#ffa170", "#9a3a00"),
        TEXT,
    ),
    row(
        "mention.role",
        "--color-xfire-mention-role",
        c2("#b59aff", "#5a32b8"),
        TEXT,
    ),
    row(
        "mention.self",
        "--color-xfire-mention-self",
        c2("#ffaea1", "#a3261b"),
        TEXT,
    ),
    Entry {
        id: "font.family",
        css: "--font-family-xfire",
        ty: TokenType::FontFamily,
        defaults: font_all("Segoe UI"),
        derivation: None,
        constraints: NONE,
        class: Class::Themeable,
    },
    Entry {
        id: "font.icons",
        css: "--font-family-icons",
        ty: TokenType::FontFamily,
        defaults: font_all("Symbols Nerd Font Mono"),
        derivation: None,
        constraints: NONE,
        class: Class::Fixed,
    },
    Entry {
        id: "size.chrome-height",
        css: "--chrome-height",
        ty: TokenType::Dimension { min: 20, max: 40 },
        defaults: Defaults {
            dark: DefaultValue::Dimension(28),
            light: DefaultValue::Dimension(28),
            hc_dark: DefaultValue::Dimension(28),
            hc_light: DefaultValue::Dimension(28),
        },
        derivation: None,
        constraints: NONE,
        class: Class::Themeable,
    },
];
