//! Xfire's content-sized text (plan SK15b): a `Class=Text
//! Stretch=ToContent` component (the menu titles, Onyx `components.xml:268-273`;
//! Combat `combat.ini:188-196`; the tab words) is as wide as its string,
//! and the tiles chained on its edge (`X="XfireMenuText.RIGHT+10"`, Onyx
//! `mainwindow.xml:42`) follow that width. The pack's strings are resource
//! ids of the Xfire binary and the Host serves its own words over them
//! ([`super::titles`]: "Rekindle", "Tools", "Help"; the views' names), so
//! the importer records each such node's limits ([`ContentFit`]) and keeps
//! each chain on it symbolic ([`Anchor::After`]), and the Host, at serve
//! time ([`serve`]), sizes the node to the words it serves and resolves
//! every chain: a stored pack never bakes the Host's words, and the shell
//! still receives numeric spans.

use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_tree::{Anchor, ContentFit, NodeRole, NodeSpan, SkinNode, SkinTree};

use crate::import::tree::Node;
use crate::import::ImportError;

use super::import::bad;

/// The shell's line box for the name (12 px bold) and for an 11 px label,
/// px (`skin-tree.css` `.skin-name`, `.skin-node-label`).
const NAME_LINE: u16 = 15;
const LABEL_LINE: u16 = 14;

/// The width of `word` in the shell's label face (`skin-tree.css`
/// `.skin-node-label`, Tahoma bold) at its 10 px floor, px: Tahoma Bold's
/// advances at 10 px, rounded up (narrow i/l/j/t/f/r 4, wide m/w 9,
/// capitals 7, the rest 6), so a box this wide never cuts the word.
#[must_use]
pub fn word_width(word: &str) -> i32 {
    word.chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | 't' | 'f' | 'r' | ' ' => 4,
            'm' | 'w' | 'M' | 'W' => 9,
            c if c.is_ascii_uppercase() => 7,
            _ => 6,
        })
        .sum()
}

/// Whether `component` is a `Class=Text Stretch=ToContent` component.
fn to_content(component: Option<&Node>) -> bool {
    let is = |k: &str, v: &str| {
        component
            .and_then(|c| c.attr_ci(k))
            .is_some_and(|a| a.trim().eq_ignore_ascii_case(v))
    };
    is("Class", "Text") && is("Stretch", "ToContent")
}

/// The content fit of a tile of `component` drawing no art of its own: its
/// `MinWidth` and `MaxWidth`; `None` for any other tile.
pub(super) fn fit_of(component: Option<&Node>, has_art: bool) -> Option<ContentFit> {
    (to_content(component) && !has_art).then(|| ContentFit {
        min: super::import::limit_of(component, "MinWidth").unwrap_or(0),
        max: super::import::limit_of(component, "MaxWidth"),
    })
}

/// One tile of a level as [`chains`] reads it: its id, whether it is a
/// content text, and its chain on each axis ([`super::anchor::chain`]).
pub(super) struct Link<'t> {
    pub(super) id: &'t str,
    pub(super) fit: bool,
    pub(super) chain: [Option<(&'t str, i32)>; 2],
}

/// Keeps symbolic each chain on a content text, or on a tile chained on
/// one (Onyx's `HelpMenuText` on `ToolsMenuText` on `XfireMenuText`): the
/// span a tile of `links` was placed at (`spans`, by index) becomes an
/// [`Anchor::After`] when it starts at the far edge of an earlier sibling
/// of the level that moves with a served width. A sibling laid after it
/// keeps the numeric span ([`SkinTree::boxes`] places in tree order).
pub(super) fn chains(links: &[Link<'_>], spans: &mut [(NodeSpan, NodeSpan)]) {
    let mut moves: Vec<bool> = links.iter().map(|l| l.fit).collect();
    for (i, l) in links.iter().enumerate() {
        for (ax, chain) in l.chain.iter().enumerate() {
            let Some((sib, by)) = chain else { continue };
            let Some(j) = links[..i]
                .iter()
                .position(|s| s.id.eq_ignore_ascii_case(sib))
            else {
                continue;
            };
            let span = if ax == 0 {
                &mut spans[i].0
            } else {
                &mut spans[i].1
            };
            let (NodeSpan::Fixed { at, .. }, true, Ok(gap)) = (span, moves[j], i16::try_from(*by))
            else {
                continue;
            };
            if matches!(at, Anchor::Start(_)) {
                *at = Anchor::After {
                    sibling: links[j].id.to_owned(),
                    gap,
                };
                moves[i] = true;
            }
        }
    }
}

/// `sib`'s far edge plus `gap` as an anchor of a span `own` px long in the
/// same parent, whatever the parent's length; `None` for an unresolved
/// sibling or a value out of range.
fn past(sib: &NodeSpan, gap: i16, own: u16) -> Option<Anchor> {
    let i = |v: i32| i16::try_from(v).ok();
    let (g, own) = (i32::from(gap), i32::from(own));
    match sib {
        NodeSpan::Fixed {
            at: Anchor::Start(o),
            size,
        } => Some(Anchor::Start(i(i32::from(*o) + i32::from(*size) + g)?)),
        NodeSpan::Fixed {
            at: Anchor::End(o), ..
        }
        | NodeSpan::Stretch { end: o, .. } => Some(Anchor::End(i(i32::from(*o) - g - own)?)),
        // The same centred frame, widened at its end by the size change,
        // starts at the sibling's start moved by its size and the gap.
        NodeSpan::Fixed {
            at: Anchor::Centre { by, before, after },
            size,
        } => Some(Anchor::Centre {
            by: i(i32::from(*by) + i32::from(*size) + g)?,
            before: *before,
            after: i(i32::from(*after) + own - i32::from(*size))?,
        }),
        NodeSpan::Fixed {
            at: Anchor::After { .. },
            ..
        } => None,
    }
}

/// A chain [`resolve`] replaced: the node, its axis (0 x, 1 y), the
/// [`Anchor::After`] and the anchor it became.
pub(super) type ResolvedChains = Vec<(String, usize, Anchor, Anchor)>;

/// Resolves every [`Anchor::After`] under `nodes` against its sibling,
/// level by level in tree order (each names an earlier sibling, already
/// resolved), recording each in `held`.
fn resolve(nodes: &mut [SkinNode], held: &mut ResolvedChains) {
    for i in 0..nodes.len() {
        for ax in 0..2 {
            let span = if ax == 0 { &nodes[i].x } else { &nodes[i].y };
            let (Some((sib, gap)), NodeSpan::Fixed { size, .. }) = (span.after(), span) else {
                continue;
            };
            let (sib, own) = (sib.to_owned(), *size);
            let found = nodes[..i].iter().find(|n| n.id.eq_ignore_ascii_case(&sib));
            let Some(at) = found.and_then(|n| past(if ax == 0 { &n.x } else { &n.y }, gap, own))
            else {
                continue;
            };
            let n = &mut nodes[i];
            if let NodeSpan::Fixed { at: slot, .. } = if ax == 0 { &mut n.x } else { &mut n.y } {
                held.push((n.id.clone(), ax, std::mem::replace(slot, at.clone()), at));
            }
        }
        resolve(&mut nodes[i].children, held);
    }
}

/// Resolves `tree`'s chains for the importer's passes, which lay boxes
/// with the import widths; [`restore`] puts them back.
pub(super) fn hold(tree: &mut SkinTree) -> ResolvedChains {
    let mut held = ResolvedChains::new();
    resolve(&mut tree.root, &mut held);
    held
}

fn find<'a>(nodes: &'a mut [SkinNode], id: &str) -> Option<&'a mut SkinNode> {
    for n in nodes {
        if n.id == id {
            return Some(n);
        }
        if let Some(found) = find(&mut n.children, id) {
            return Some(found);
        }
    }
    None
}

/// Puts back the chains [`hold`] resolved.
///
/// # Errors
/// A pass that moved or removed a chained node: its place is no longer
/// its chain's.
pub(super) fn restore(tree: &mut SkinTree, held: ResolvedChains) -> Result<(), ImportError> {
    for (id, ax, after, laid) in held {
        let n = find(&mut tree.root, &id).ok_or_else(|| bad(format!("{id}: chain lost")))?;
        match if ax == 0 { &mut n.x } else { &mut n.y } {
            NodeSpan::Fixed { at, .. } if *at == laid => *at = after,
            _ => return Err(bad(format!("{id}: a pass moved a chained node"))),
        }
    }
    Ok(())
}

/// The words the Host serves over `n` ([`super::titles`]): a menu title's
/// ([`rekindle_types::dto::skin_action::EngineMenu::title`]) or a text
/// tab's view name ([`rekindle_types::dto::skin_tree::ViewKey::title`]);
/// `None` for words only the running app knows (the user's name, plan
/// SK15b-2).
fn served_words(n: &SkinNode) -> Option<&'static str> {
    match n.role {
        NodeRole::Tab(v) => Some(v.title()),
        NodeRole::MenuTitle => super::titles::menu_of(n).map(|m| m.title().ours),
        _ => None,
    }
}

/// The width a content text with `fit` serving `words` takes: the words'
/// plus 1 px, at least `fit.min`, at most `fit.max`.
#[must_use]
pub fn width(fit: ContentFit, words: &str) -> u16 {
    let w = u16::try_from(word_width(words) + 1)
        .unwrap_or(u16::MAX)
        .max(fit.min);
    fit.max.map_or(w, |m| w.min(m))
}

/// Sizes every content text under `nodes` to its served words, and one
/// with no height a line tall: a sibling chained on its bottom edge clears
/// the text.
fn size(nodes: &mut [SkinNode]) {
    for n in nodes {
        if let Some(fit) = n.fit {
            let w = served_words(n).map(|words| width(fit, words));
            if let (NodeSpan::Fixed { size, .. }, Some(w)) = (&mut n.x, w) {
                *size = w;
            }
            let line = if n.role == NodeRole::NameField {
                NAME_LINE
            } else {
                LABEL_LINE
            };
            if let NodeSpan::Fixed { size: h @ 0, .. } = &mut n.y {
                *h = line;
            }
        }
        size(&mut n.children);
    }
}

fn serve_tree(tree: &mut SkinTree, _at: (i32, i32)) {
    size(&mut tree.root);
    resolve(&mut tree.root, &mut ResolvedChains::new());
}

/// Sizes every content text of `layout`'s tree and popups to the words
/// the Host serves over it and resolves every chain into a numeric span
/// (see the module doc). Runs before every other serve pass
/// ([`crate::manifest::place`]), which all lay numeric spans.
pub fn serve(layout: &mut WindowLayout) {
    super::titles::each_laid(layout, serve_tree);
}
