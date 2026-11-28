//! SVG icons rasterised to PNG at import (plan E7.7.3 item 5;
//! `c-security.md` §2.2): SVG is an input format only, never served.
//!
//! In order, before anything is drawn:
//! 1. ≤ [`MAX_SVG_BYTES`], UTF-8;
//! 2. a pre-scan with `roxmltree` (the reader usvg uses): no DTD (so no
//!    entity reaches usvg, which parses with `allow_dtd: true`,
//!    `usvg-0.45.1/src/parser/mod.rs:110-111`), ≤ [`MAX_SVG_NODES`]
//!    nodes (`ParsingOptions::nodes_limit`), no `<text>` (resvg is built
//!    without its `text` feature: no font database, no system fonts) and
//!    no `<?xml-stylesheet?>`;
//! 3. usvg parses with a resolver that refuses **every** `<image>` href:
//!    `usvg::Options::image_href_resolver` holds an `ImageHrefResolver`
//!    whose `resolve_data` and `resolve_string` both record the reference
//!    and return `None` (`usvg-0.45.1/src/parser/image.rs:28-37`, called at
//!    `:298-300`). The default string resolver opens the href as a local
//!    path with `std::fs::read` (`skins/raw/c/vendors/usvg-image.rs:85-99`);
//!    ours never touches the file system;
//! 4. the tree's size is checked against [`MAX_IMAGE_SIDE`] and
//!    [`MAX_IMAGE_PIXELS`]; only then is a pixmap allocated and resvg draws.

use std::sync::mpsc::{channel, Sender};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{ImageHrefResolver, Options, Tree};

use crate::limits::{MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};

/// The largest SVG input (`c-security.md` §8.6).
pub const MAX_SVG_BYTES: usize = 1024 * 1024;
/// The most XML nodes an SVG may hold (`c-security.md` §8.6).
pub const MAX_SVG_NODES: u32 = 20_000;

/// An SVG drawn to PNG, and every reference the resolver refused.
#[derive(Debug)]
pub struct Raster {
    pub png: Vec<u8>,
    pub refused: Vec<String>,
}

/// The pre-scan: size, encoding, DTD, node count, text, stylesheets.
fn prescan(bytes: &[u8]) -> Result<&str, String> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err(format!("{} bytes is past {MAX_SVG_BYTES}", bytes.len()));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned())?;
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_SVG_NODES,
    };
    let doc = roxmltree::Document::parse_with_options(text, options).map_err(|e| e.to_string())?;
    for node in doc.descendants() {
        if node.is_element() && node.tag_name().name() == "text" {
            return Err("a <text> element: text needs fonts, which are not loaded".to_owned());
        }
        if node.pi().is_some_and(|pi| pi.target == "xml-stylesheet") {
            return Err("an <?xml-stylesheet?> instruction".to_owned());
        }
    }
    Ok(text)
}

/// The options usvg parses with: every image href refused and sent to
/// `refused` (a `Sender` is `Send + Sync`, as the resolver must be).
fn refusing_options(refused: &Sender<String>) -> Options<'static> {
    let data = refused.clone();
    let string = refused.clone();
    Options {
        image_href_resolver: ImageHrefResolver {
            resolve_data: Box::new(move |mime, _, _| {
                // The receiver outlives the parse; a send cannot fail.
                let _ = data.send(format!("data:{mime}"));
                None
            }),
            resolve_string: Box::new(move |href, _| {
                let _ = string.send(href.to_owned());
                None
            }),
        },
        ..Options::default()
    }
}

/// The parsed tree and the refused references, without drawing.
///
/// # Errors
/// What the pre-scan refuses, or a usvg parse error.
pub fn parse(bytes: &[u8]) -> Result<(Tree, Vec<String>), String> {
    let text = prescan(bytes)?;
    let (tx, rx) = channel();
    let tree = Tree::from_str(text, &refusing_options(&tx)).map_err(|e| e.to_string())?;
    Ok((tree, rx.try_iter().collect()))
}

/// The pixel size `tree` is drawn at, or why it is refused.
///
/// # Errors
/// A side past [`MAX_IMAGE_SIDE`] or an area past
/// [`MAX_IMAGE_PIXELS`].
pub fn raster_size(tree: &Tree) -> Result<(u32, u32), String> {
    // Rounded and saturated to 1..=u32::MAX by tiny-skia (`Size::to_int_size`).
    let size = tree.size().to_int_size();
    let (w, h) = (size.width(), size.height());
    if w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        return Err(format!("{w}x{h}: a side is outside 1..={MAX_IMAGE_SIDE}"));
    }
    if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
        return Err(format!("{w}x{h} is past {MAX_IMAGE_PIXELS} pixels"));
    }
    Ok((w, h))
}

/// Draws an SVG to PNG at its own size.
///
/// # Errors
/// What [`parse`] or [`raster_size`] refuses, or a PNG encoding failure.
pub fn rasterise(bytes: &[u8]) -> Result<Raster, String> {
    let (tree, refused) = parse(bytes)?;
    let (w, h) = raster_size(&tree)?;
    let mut pixmap = Pixmap::new(w, h).ok_or_else(|| format!("no {w}x{h} pixmap"))?;
    resvg::render(&tree, Transform::default(), &mut pixmap.as_mut());
    let png = pixmap.encode_png().map_err(|e| e.to_string())?;
    Ok(Raster { png, refused })
}
