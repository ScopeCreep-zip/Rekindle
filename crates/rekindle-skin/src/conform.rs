//! The conforming alternate of Symbiosis (plan SK14; WCAG 2.2 §5.2.1
//! "Conforming Alternate Version", glossary note 7): the Xfire skins stay
//! pixel-faithful, so the built-ins' few undersized controls are served as
//! the packs lay them (`TargetRule::Short`), and one built-in,
//! [`ID`] ("Symbiosis (Accessible)"), is Symbiosis re-laid so no control is
//! under 24 x 24 px (SC 2.5.8). The picker shows every skin's count
//! ([`short_targets`]); an imported pack is not vouched for.
//!
//! The alternate is never a pack of its own: [`crate::builtin::validated`]
//! refuses any bytes but a committed pack's, so it is derived at serve time
//! from Symbiosis's committed bytes. It has its own `skin://` address
//! ([`address`]: BLAKE3 in derive-key mode over Symbiosis's address under
//! [`DOMAIN`]), so the webview never confuses the two stylesheets, and
//! [`served`] lays it whenever a package is served at that address.

use std::sync::OnceLock;

use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule, SkinWindow};
use rekindle_types::dto::skin_tree::{Anchor, NodeSpan, SkinNode, TargetRule};

use crate::package::{self, Package};
use crate::{builtin, manifest};

/// The alternate's skin id.
pub const ID: &str = "builtin:symbiosis-access";

/// The built-in it is derived from.
pub const SOURCE: &str = builtin::DEFAULT;

/// Its display name, the served manifest's `name`.
pub const NAME: &str = "Symbiosis (Accessible)";

/// The BLAKE3 derive-key context of its address.
pub const DOMAIN: &str = "rekindle-skin 2026-11-23 conforming alternate builtin:symbiosis-access";

/// Symbiosis's committed pack.
fn source() -> &'static [u8] {
    builtin::find(SOURCE).map_or(&[], |(_, b)| b.bytes)
}

/// The alternate's `skin://` address: BLAKE3 (derive-key, [`DOMAIN`]) of
/// Symbiosis's address, 64 lowercase hex.
#[must_use]
pub fn address() -> &'static str {
    static ADDRESS: OnceLock<String> = OnceLock::new();
    ADDRESS.get_or_init(|| {
        let symbiosis = blake3::hash(source()).to_hex();
        blake3::Hash::from(blake3::derive_key(DOMAIN, symbiosis.as_bytes()))
            .to_hex()
            .to_string()
    })
}

/// Whether `skin_id` names a built-in: a committed pack or the alternate.
#[must_use]
pub fn is_builtin(skin_id: &str) -> bool {
    skin_id == ID || builtin::find(skin_id).is_some()
}

/// The pack a built-in id is served from: Symbiosis for the alternate.
#[must_use]
pub fn pack_id(skin_id: &str) -> &str {
    if skin_id == ID {
        SOURCE
    } else {
        skin_id
    }
}

/// One node moved: its id, its span as Symbiosis lays it, its new span.
struct Move {
    id: &'static str,
    from: NodeSpan,
    to: NodeSpan,
}

const fn end(offset: i16, size: u16) -> NodeSpan {
    NodeSpan::Fixed {
        at: Anchor::End(offset),
        size,
    }
}

/// Symbiosis re-laid (x spans only), measured at its served minimum.
/// Its main window (368 x 555; `InfoBar` spans x 5–350) and its chat
/// window (332 wide; `InfoBar` spans x 5–314) carry the same title row
/// (`MainWindow.xml:55-57`, `ChatWindow.xml:64-66`: `ButtonMinimize`,
/// `ButtonMaximize`, `ButtonClose` at `JustX=Right` 49, 25 and 5 in an
/// `InfoBar` indented 5 and 18), so the same rule moves it in each:
///
/// - The title buttons (18 x 16 art, 3 px apart in Symbiosis) end 5 px
///   from `InfoBar`'s end, so their row has 62 px for three 24 px
///   targets, and `ButtonClose`'s target reaches the top-right grip's
///   24 x 24 corner (the window's last 24 px, y < 24), which then loses
///   it. Each is moved left so the three arts sit 6 px apart, their
///   targets 24 px wide and edge to edge, `ButtonClose`'s ending where
///   the corner begins: in the main window art x 262, 286, 310 (were
///   283, 307, 327), in the chat window 226, 250, 274 (were 247, 271,
///   291). The main window's logo ends at x 130 and the chat window's
///   user name box at x 221 (`ChatUserContainer`, `IndentRight="93"`), so
///   the row meets nothing.
/// - `Status` (the status text in the main window, ending 18 px from its
///   bar's end) abuts `StatusChanger` (gap 0), whose target could take
///   only 21 px: it ends 3 px sooner, so the changer's target is 24 x 24
///   (x 329–353).
///
/// Every node, trigger and art is kept; only these spans move, in every
/// window that holds the node.
fn moves() -> [Move; 4] {
    [
        Move {
            id: "ButtonMinimize",
            from: end(49, 18),
            to: end(70, 18),
        },
        Move {
            id: "ButtonMaximize",
            from: end(25, 18),
            to: end(46, 18),
        },
        Move {
            id: "ButtonClose",
            from: end(5, 18),
            to: end(22, 18),
        },
        Move {
            id: "Status",
            from: NodeSpan::Stretch { start: 5, end: 18 },
            to: NodeSpan::Stretch { start: 5, end: 21 },
        },
    ]
}

fn node_mut<'a>(nodes: &'a mut [SkinNode], id: &str) -> Option<&'a mut SkinNode> {
    nodes.iter_mut().find_map(|n| {
        if n.id == id {
            Some(n)
        } else {
            node_mut(&mut n.children, id)
        }
    })
}

/// Re-lays Symbiosis's parsed manifest as the alternate ([`moves`]), before
/// the Host's placements, so the text-box, inset and target passes all lay
/// the moved boxes. Each move applies in every window and popup holding
/// its node; a node not laid as Symbiosis lays it, or a move no window
/// holds, is refused: the alternate is this relayout of that pack, never a
/// guess.
///
/// # Errors
/// A moved node is missing from every window or not where Symbiosis lays
/// it.
pub fn relayout(m: &mut SkinManifest) -> Result<(), SkinError> {
    let refuse = |w: SkinWindow, id: &str| {
        SkinError::new(
            package::MANIFEST,
            SkinRule::BadValue {
                field: format!("windows.{w:?}.tree.{id}.x").to_ascii_lowercase(),
                expected: "Symbiosis's span, which the accessible alternate moves".to_owned(),
            },
        )
    };
    for mv in moves() {
        let mut held = false;
        for (w, l) in &mut m.windows {
            let roots = l
                .tree
                .iter_mut()
                .map(|t| &mut t.root)
                .chain(l.popups.values_mut().map(|p| &mut p.tree.root));
            for root in roots {
                if let Some(n) = node_mut(root, mv.id) {
                    if n.x != mv.from {
                        return Err(refuse(*w, mv.id));
                    }
                    n.x = mv.to.clone();
                    held = true;
                }
            }
        }
        if !held {
            return Err(refuse(SkinWindow::Main, mv.id));
        }
    }
    NAME.clone_into(&mut m.name);
    Ok(())
}

/// The manifest the Host serves for the canonical package `bytes` read as
/// `p` at `address`: the alternate's ([`relayout`], then the placements)
/// at [`address`], which only Symbiosis's committed bytes may be served
/// at; else [`manifest::served`].
///
/// # Errors
/// As [`manifest::served`]; other bytes at the alternate's address.
pub fn served(bytes: &[u8], p: &Package<'_>, address: &str) -> Result<SkinManifest, SkinError> {
    if address != self::address() {
        return manifest::served(p);
    }
    if bytes != source() {
        return Err(SkinError::new(
            SkinError::PACKAGE,
            SkinRule::BadValue {
                field: "address".to_owned(),
                expected: "Symbiosis's committed pack at the accessible alternate's address"
                    .to_owned(),
            },
        ));
    }
    let mut m = manifest::parse(p)?;
    relayout(&mut m)?;
    manifest::place(&mut m, p)?;
    Ok(m)
}

fn shorts(nodes: &[SkinNode]) -> u32 {
    nodes
        .iter()
        .map(|n| {
            u32::from(n.target.is_some_and(|t| t.rule == TargetRule::Short)) + shorts(&n.children)
        })
        .sum()
}

/// The controls `m` serves under 24 px with no spacing exception
/// (`TargetRule::Short`, grips included) over every window's tree and
/// popups.
#[must_use]
pub fn short_targets(m: &SkinManifest) -> u32 {
    m.windows
        .values()
        .map(|l| {
            l.tree.as_ref().map_or(0, |t| shorts(&t.root))
                + l.popups.values().map(|p| shorts(&p.tree.root)).sum::<u32>()
        })
        .sum()
}

/// [`short_targets`] of the canonical package `bytes` as served (a library
/// skin: from its bytes, never its stored manifest).
///
/// # Errors
/// The package or its manifest does not read.
pub fn short_targets_of(bytes: &[u8]) -> Result<u32, SkinError> {
    let p = package::read(bytes)?;
    Ok(short_targets(&manifest::served(&p)?))
}

/// [`short_targets`] of the built-in `skin_id` as served, once per process;
/// `None` when it names no built-in.
///
/// # Errors
/// The pack does not read (never for a committed pack).
pub fn builtin_short_targets(skin_id: &str) -> Result<Option<u32>, SkinError> {
    static COUNTS: [OnceLock<u32>; builtin::PACKS.len() + 1] =
        [const { OnceLock::new() }; builtin::PACKS.len() + 1];
    let (slot, address) = if skin_id == ID {
        (builtin::PACKS.len(), self::address().to_owned())
    } else if let Some((i, b)) = builtin::find(skin_id) {
        (i, blake3::hash(b.bytes).to_hex().to_string())
    } else {
        return Ok(None);
    };
    if let Some(n) = COUNTS[slot].get() {
        return Ok(Some(*n));
    }
    let bytes = builtin::find(pack_id(skin_id)).map_or(&[][..], |(_, b)| b.bytes);
    let p = package::read(bytes)?;
    let n = short_targets(&served(bytes, &p, &address)?);
    Ok(Some(*COUNTS[slot].get_or_init(|| n)))
}

#[cfg(test)]
#[path = "conform_tests.rs"]
mod tests;
