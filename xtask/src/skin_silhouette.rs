//! `cargo xtask skin-silhouette` (plan E7.14, the shaped-window spec on
//! SK0): the silhouette a built-in skin cuts a window to, for the real-app
//! spec to measure the window's drawing against. It is the shaper's own
//! computation (`src-tauri/src/windows/shape/skin.rs` `compute`):
//! `rekindle_skin::serve::frames` of the pack, the frame the window kind
//! wears (the buddy list `main`, a conversation `chat`), its tile masks,
//! and `rekindle_skin::shape::silhouette` at the window's logical size and
//! scale. Dev tooling only: the page never receives a silhouette
//! (`check-no-shape-ipc`).
//!
//! Prints `{"alphaOpaque":128,"rects":[{"x","y","width","height"}…]}`, the
//! rects in device pixels, or `"rects":null` when the skin lays no frame for
//! that window (it keeps its rectangle). `alphaOpaque` is the alpha from
//! which a pixel is part of the window (`rekindle_skin::shape::ALPHA_OPAQUE`),
//! applied to the composited alpha of every layer laid on it.
//! `content` is the layout's content rectangle at that size (the pack's
//! views box, `rekindle_skin::shape::content_area`, plan SK2), in logical
//! px, for the spec to hold the page's content box to; `null` when the
//! skin lays no frame.

use anyhow::{anyhow, Context, Result};
use clap::Args as ClapArgs;
use serde::Serialize;

use rekindle_skin::builtin::PACKS;
use rekindle_skin::serve::frames;
use rekindle_skin::shape::{content_area, silhouette, Area, Mask, ALPHA_OPAQUE};
use rekindle_types::dto::skin::SkinWindow;

/// `cargo xtask skin-silhouette`'s arguments.
#[derive(ClapArgs)]
pub(crate) struct Args {
    /// The pack id in `rekindle_skin::builtin::PACKS` (`xfire-classic`).
    #[arg(long)]
    pack: String,
    /// The frame: `main` (the buddy list) or `chat`.
    #[arg(long)]
    window: String,
    /// The window's logical width.
    #[arg(long)]
    width: u32,
    /// The window's logical height.
    #[arg(long)]
    height: u32,
    /// The scale in thousandths (1000 at 1x, 2000 at 2x).
    #[arg(long)]
    scale: u32,
}

#[derive(Serialize)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    alpha_opaque: u8,
    rects: Option<Vec<Rect>>,
    content: Option<Rect>,
}

/// A logical-px area as a rect, negative edges and sizes clamped to 0.
fn rect(a: Area) -> Rect {
    let u = |v: i64| u32::try_from(v.max(0)).unwrap_or(u32::MAX);
    Rect {
        x: u(a.0),
        y: u(a.1),
        width: u(a.2),
        height: u(a.3),
    }
}

/// The frame `window` names (`main` or `chat`, `SkinWindow`'s camelCase).
fn frame(window: &str) -> Result<SkinWindow> {
    match window {
        "main" => Ok(SkinWindow::Main),
        "chat" => Ok(SkinWindow::Chat),
        "community" => Ok(SkinWindow::Community),
        other => Err(anyhow!(
            "window {other:?} is not a skin frame (main, chat or community)"
        )),
    }
}

/// Print the silhouette of `pack`'s `window` frame at `width × height`
/// logical pixels and `scale` per mille.
pub(crate) fn skin_silhouette(args: &Args) -> Result<()> {
    let Args {
        pack,
        window,
        width,
        height,
        scale,
    } = args;
    let (width, height, scale) = (*width, *height, *scale);
    let wanted = frame(window)?;
    let builtin = PACKS
        .iter()
        .find(|b| b.id == pack)
        .ok_or_else(|| anyhow!("no built-in pack {pack:?} in rekindle_skin::builtin::PACKS"))?;
    let address = blake3::hash(builtin.bytes).to_hex();
    let windows =
        frames(builtin.bytes, address.as_str()).with_context(|| format!("frames of {pack}"))?;
    let mut content = None;
    let rects = match windows.into_iter().find(|w| w.window == wanted) {
        None => None,
        Some(w) => {
            content = Some(rect(content_area(&w.layout, width, height)));
            let masks = w
                .masks
                .into_iter()
                .map(|m| Mask::from_parts(m.width, m.height, m.bits, m.faint))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| anyhow!("{pack}: a malformed tile mask in {window}"))?;
            let rects = silhouette(&w.layout, &masks, width, height, scale).map_err(|e| {
                anyhow!("{pack} {window} at {width}x{height}, scale {scale}: {e:?}")
            })?;
            Some(
                rects
                    .into_iter()
                    .map(|r| Rect {
                        x: r.x,
                        y: r.y,
                        width: r.width,
                        height: r.height,
                    })
                    .collect(),
            )
        }
    };
    println!(
        "{}",
        serde_json::to_string(&Answer {
            alpha_opaque: ALPHA_OPAQUE,
            rects,
            content,
        })?
    );
    Ok(())
}
