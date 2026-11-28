//! The colour roles a skin's components name (plan E7.7.7, the Linux skin
//! survey's item (b): six fan skins rendered the base accent and surfaces).
//!
//! An Xfire colour name is the skin author's own label; the client never
//! read a role from it. The role is the reference: the component (XML
//! `<Component Name=…>`, INI `[section]`) and the attribute that names the
//! colour (`<Component Color=…>`/`ColorGroup=`, `<Text color_selected=…>`,
//! INI `ColorGroup=`/`Text.color_selected=`). Symbiosis labels its colours
//! by role (`ListBGCG`), which [`super::colours::NAMES`] reads; Onyx calls
//! the same list ground `LBGCG`, StealthX `MainListBackColor`, Xglow
//! `Base`. So after the name table, every reference whose component and
//! attribute hold a role in [`COMPONENT_ROLES`] sets that token, unless the name
//! table already set it (a role-named colour wins) or an earlier reference
//! did (file order).
//!
//! The accent: a list's selection is a ground (`color_selected`) and a text
//! (`color_selectedtext`). Symbiosis's selection ground is its blue, the
//! accent. The fan skins select with a near-black ground and their hue on
//! the text (Onyx `Selected` 25,25,25 under green 114,255,0); a neutral
//! fill cannot mark a button, a tab or a focus apart from the panels, so
//! when the selection ground is neutral (channel spread under
//! [`NEUTRAL_SPREAD`]) the selection text's hue is the accent; a
//! selection with no hue in either (Alloy's grey on black) names none. The
//! accent is the selection's chromatic colour (the text's hue when the
//! ground is neutral), on `accent.fill` and `accent.text`. The walk moves
//! lightness only, so a hue the danger rules refuse as a fill (D1-D3: a red
//! accent, or one deutan vision confuses with danger red) leaves
//! `accent.fill` at the base, recorded in the `NOTICE`, and the hue on
//! `accent.text`. Every value then goes through the import's pair-model
//! walk like any other token.

use std::collections::BTreeMap;

use rekindle_types::dto::skin::TokenValue;

use super::super::tree::Node;
use super::super::walk;
use super::colours::absolute;
use super::{descendants, ColourRow};
use crate::access;
use crate::registry::{entry_in, Base, ENTRIES};
use rekindle_types::dto::skin::{SkinBase, SkinContrast};

/// A selection ground whose channels spread less than this is neutral.
pub const NEUTRAL_SPREAD: i16 = 24;

/// The list components (their `Text` selection colours are the accent).
const LISTS: &[&str] = &["BuddyView", "XfireView"];

/// `(components, attribute, token)`; `color_selected`/`color_selectedtext`
/// are the accent ([`accent`]). Attributes compare ASCII-case-insensitively.
pub const COMPONENT_ROLES: &[(&[&str], &str, &str)] = &[
    (LISTS, "colorgroup", "bg.primary"),
    (LISTS, "color", "bg.primary"),
    (&["ChatArea", "GroupChatArea"], "colorgroup", "bg.panel"),
    (&["ChatArea", "GroupChatArea"], "color", "bg.panel"),
    (&["ChatEdit", "GroupChatEdit"], "colorgroup", "bg.input"),
    (&["ChatEdit", "GroupChatEdit"], "color", "bg.input"),
    (INFO, "colorgroup", "bg.secondary"),
    (INFO, "color", "bg.secondary"),
    (INFO, "text.color_hyperlink", "link"),
    (&["StatusEdit", "Status"], "text.background", "bg.status"),
    (&["StatusEdit", "Status"], "text.color", "text.status"),
];

const INFO: &[&str] = &[
    "InfoView",
    "GroupInfoView",
    "InfoViewTile",
    "InfoInfoView",
    "GroupChatInfoView",
    "MainInfoViewContent",
    "ChatInfoViewContent",
];

/// One reference: `[component] attribute=colour`. A `Text` child's
/// attribute is `text.<attr>` (the INI spelling, `Text.color_selected`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub component: String,
    pub attr: String,
    pub colour: String,
}

/// What one reference did, for the `NOTICE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefsApplied {
    pub reference: Ref,
    pub token: &'static str,
    /// `None`: set; else why the token kept its value.
    pub kept: Option<&'static str>,
}

impl RefsApplied {
    /// The `NOTICE` line.
    #[must_use]
    pub fn describe(&self) -> String {
        let r = &self.reference;
        let what = self.kept.map_or_else(
            || format!("token {}", self.token),
            |why| format!("{} kept: {why}", self.token),
        );
        format!("[{}] {}={} -> {what}", r.component, r.attr, r.colour)
    }
}

/// Every reference of the XML components, in file order: a `Component`'s
/// own `Color`/`ColorGroup` and its `Text` children's attributes.
#[must_use]
pub fn xml_refs(components: &[Node]) -> Vec<Ref> {
    let mut out = Vec::new();
    for c in components.iter().flat_map(|f| descendants(f, "Component")) {
        let Some(name) = c.attr_ci("Name") else {
            continue;
        };
        let mut push = |attr: String, colour: &str| {
            out.push(Ref {
                component: name.to_owned(),
                attr,
                colour: colour.to_owned(),
            });
        };
        for (k, v) in &c.attrs {
            push(k.to_ascii_lowercase(), v);
        }
        for t in c
            .children
            .iter()
            .filter(|n| n.name.eq_ignore_ascii_case("Text"))
        {
            for (k, v) in &t.attrs {
                push(format!("text.{}", k.to_ascii_lowercase()), v);
            }
        }
    }
    out
}

fn is(list: &[&str], name: &str) -> bool {
    list.iter().any(|n| n.eq_ignore_ascii_case(name))
}

fn role(r: &Ref) -> Option<&'static str> {
    COMPONENT_ROLES
        .iter()
        .find(|(cs, a, _)| is(cs, &r.component) && r.attr.eq_ignore_ascii_case(a))
        .map(|(_, _, t)| *t)
}

/// The accent references, `(token, ref)`: the first list's selection.
fn accent<'r>(refs: &'r [Ref], rows: &[ColourRow]) -> Vec<(&'static str, &'r Ref)> {
    let pick = |attr: &str| {
        refs.iter().find(|r| {
            is(LISTS, &r.component)
                && r.attr.eq_ignore_ascii_case(attr)
                && rows.iter().any(|c| c.name == r.colour)
        })
    };
    let (ground, text) = (pick("text.color_selected"), pick("text.color_selectedtext"));
    let chromatic = |r: &&Ref| {
        rows.iter().find(|c| c.name == r.colour).is_some_and(|c| {
            let (hi, lo) = (c.value.iter().max(), c.value.iter().min());
            hi.zip(lo).is_some_and(|(h, l)| h - l >= NEUTRAL_SPREAD)
        })
    };
    let hue = ground.filter(chromatic).or_else(|| text.filter(chromatic));
    [("accent.fill", hue), ("accent.text", hue)]
        .into_iter()
        .filter_map(|(t, r)| r.map(|r| (t, r)))
        .collect()
}

/// The danger fill the accent must stay apart from (D1-D3): the skin's,
/// else the registry's (one value on every base).
fn danger(tokens: &BTreeMap<String, TokenValue>) -> [u8; 3] {
    let registry = || {
        entry_in(ENTRIES, "danger.fill").map(|e| {
            e.defaults
                .on(Base::of(SkinBase::Dark, SkinContrast::Standard))
                .value()
        })
    };
    match tokens.get("danger.fill").cloned().or_else(registry) {
        Some(TokenValue::Color { r, g, b, .. }) => [r, g, b],
        _ => unreachable!("the registry holds the colour danger.fill"),
    }
}

/// Sets every role token [`COMPONENT_ROLES`] and [`accent`] find in `refs` that
/// neither the name table (`tokens` as read) nor an earlier reference set;
/// a referenced shift (a negative channel) sets nothing.
pub fn apply(
    refs: &[Ref],
    rows: &[ColourRow],
    tokens: &mut BTreeMap<String, TokenValue>,
) -> Vec<RefsApplied> {
    let named: Vec<String> = tokens.keys().cloned().collect();
    let mut out = Vec::new();
    let found = accent(refs, rows)
        .into_iter()
        .chain(refs.iter().filter_map(|r| role(r).map(|t| (t, r))));
    for (token, r) in found {
        let Some(row) = rows.iter().find(|c| c.name == r.colour) else {
            continue;
        };
        let kept = if named.iter().any(|n| n == token) {
            Some("its role-named colour wins")
        } else if tokens.contains_key(token) {
            Some("an earlier reference set it")
        } else if let Ok(rgb) = absolute(&row.name, "a referenced colour", row.value) {
            if token == "accent.fill" && !access::danger_failures(danger(tokens), rgb).is_empty() {
                out.push(RefsApplied {
                    reference: r.clone(),
                    token,
                    kept: Some("D1-D3: this hue as a fill would read as danger"),
                });
                continue;
            }
            tokens.insert(token.to_owned(), walk::color(rgb));
            None
        } else {
            Some("a signed image shift, not a colour")
        };
        out.push(RefsApplied {
            reference: r.clone(),
            token,
            kept,
        });
    }
    out
}
