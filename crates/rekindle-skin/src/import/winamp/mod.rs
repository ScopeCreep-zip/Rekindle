//! The Winamp classic `.wsz` importer (plan E7.7.3 item 3): a
//! "Winamp-flavoured Rekindle skin" per `skins/research/d1-skeuo-winamp.md`
//! §2.
//!
//! * [`archive`]: the zip, under the legacy limits; only GEN.BMP,
//!   GENEX.BMP, TITLEBAR.BMP and PLEDIT.TXT are inflated. REGION.TXT, the
//!   cursors (`.cur`/`.ani`, CVE-2007-0038), TEXT.BMP and `WINAMPMB.TXT`
//!   are never read past their names.
//! * [`bmp`]: the bounded BMP decoder, RLE8 included (only here).
//! * [`sprites`]: the 39 mapped sprites. GEN.BMP builds `window.frame`
//!   and `titlebar`, active (`_SELECTED`) as `normal` and plain as
//!   `windowInactive`; TITLEBAR's buttons and GENEX's scrollbar parts are
//!   their painted slots; GENEX's left, centre and right button pieces are
//!   joined into `button.secondary`'s one nine-slice (its pressed sprite is
//!   `down`), its `textBacking` GENEX's window background (`bg.panel`).
//! * [`pledit`]: PLEDIT.TXT owns the list tokens; GENEX's 22 colour pixels
//!   the rest ([`sprites::GENEX_COLOURS`]).
//!
//! A user import whose package fails the validator is an [`ImportDraft`]:
//! the package as imported, every failing pair with its suggested passing
//! colour (the walk's [`walk::Change`]), and the package with every
//! suggestion accepted. A refusal the walk cannot settle (stock Winamp's
//! `#ffffff` window text leaves fixed `danger.fg` failing on its fill, or
//! no lightness of `text.primary` passing on every ground) is the draft's
//! [`ImportDraft::unsettled`] failure, with no suggestion and no suggested
//! package, never an error. The editor draft type does not exist yet; this
//! is its input.

pub mod archive;
pub mod bmp;
mod notice;
pub mod pledit;
pub mod sprites;

use std::collections::BTreeMap;

use rekindle_image::{encode_png, DynamicImage, RgbaImage};
use rekindle_types::dto::skin::{
    RegionFit, RegionSpec, RegionState, SkinBase, SkinContrast, SkinLicense, SkinManifest,
    SkinSource, SkinVariant, TokenValue, SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};

use super::walk;
use super::{base_of, conclude, Control, ImportError, Tree};
pub use super::{ImportDraft, Outcome};
use crate::output::{token_file, zip};
use crate::package::MANIFEST;
use crate::regions::slot_of;
use sprites::{Sprite, Target, SPRITES};

/// The `source.format` of an imported package.
pub const WINAMP_WSZ_FORMAT: &str = "winamp-wsz";
const TOKENS: &str = "tokens.json";
const FRAME: [&str; 2] = [
    "assets/window-frame.png",
    "assets/window-frame-inactive.png",
];
const TITLE: [&str; 2] = ["assets/titlebar.png", "assets/titlebar-inactive.png"];

/// One colour the skin defines and what it became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColourRow {
    pub source: String,
    pub value: [u8; 3],
    pub target: Target,
}

/// The sprites of painted slots, grouped by slot in table order.
fn controls(decoded: &[(Sprite, RgbaImage)]) -> Vec<Control> {
    let mut out: Vec<Control> = Vec::new();
    for (sprite, img) in decoded {
        if !slot_of(sprite.region).is_some_and(|s| s.css)
            || sprites::BUTTON_PARTS.contains(&sprite.name)
        {
            continue;
        }
        let i = if let Some(i) = out.iter().position(|c| c.region == sprite.region) {
            i
        } else {
            out.push(Control {
                region: sprite.region,
                fit: RegionFit::Stretch,
                images: BTreeMap::new(),
            });
            out.len() - 1
        };
        out[i]
            .images
            .entry(sprites::state(sprite))
            .or_insert_with(|| img.clone());
    }
    // The three pieces are one image; the pressed sprite is already whole.
    if let Some(img) = sprites::button(decoded) {
        if let Some(c) = out.iter_mut().find(|c| c.region == sprites::BUTTON) {
            c.images.insert(RegionState::Normal, img);
        } else {
            out.push(Control {
                region: sprites::BUTTON,
                fit: RegionFit::Stretch,
                images: BTreeMap::from([(RegionState::Normal, img)]),
            });
        }
    }
    for c in &mut out {
        if c.region == sprites::BUTTON {
            c.fit = RegionFit::NineSlice(sprites::BUTTON_INSETS);
        }
    }
    out.retain(|c| c.images.contains_key(&RegionState::Normal));
    out
}

/// What the importer read, before the package is built.
#[derive(Debug)]
pub struct Read {
    pub colours: Vec<ColourRow>,
    pub tokens: BTreeMap<String, TokenValue>,
    /// Every mapped sprite and whether its sheet held it.
    pub sprites: Vec<(Sprite, bool)>,
    /// `window.frame` and `titlebar`, each [active, inactive].
    pub frame: Option<[RgbaImage; 2]>,
    pub title: Option<[RgbaImage; 2]>,
    /// The painted control slots (titlebar buttons, scrollbar parts): each
    /// slot's images by state, from TITLEBAR.BMP and GENEX.BMP.
    pub controls: Vec<Control>,
    /// PLEDIT.TXT keys that are not colours we know.
    pub unknown: Vec<String>,
    /// PLEDIT.TXT `Font=` and the allowed face it names.
    pub font: Option<pledit::Font>,
    /// Entries never read past their names.
    pub ignored: Vec<String>,
}

fn sheet(tree: &Tree, name: &str) -> Result<Option<RgbaImage>, ImportError> {
    tree.get(name).map(|b| bmp::decode(name, b)).transpose()
}

type Rows = (Vec<ColourRow>, Vec<String>, Option<pledit::Font>);

fn colour_rows(tree: &Tree, genex: Option<&RgbaImage>) -> Rows {
    let mut rows = Vec::new();
    let mut unknown = Vec::new();
    let mut font = None;
    if let Some(bytes) = tree.get("pledit.txt") {
        for line in pledit::lines(bytes) {
            if line.key.eq_ignore_ascii_case(pledit::FONT_KEY) {
                font = Some(pledit::font(&line.value));
                continue;
            }
            match (pledit::target(&line.key), pledit::rgb(&line.value)) {
                (Some(target), Some(value)) => rows.push(ColourRow {
                    source: format!("PLEDIT.TXT {}", line.key),
                    value,
                    target,
                }),
                _ => unknown.push(format!("{}={}", line.key, line.value)),
            }
        }
    }
    if let Some(g) = genex.filter(|g| g.width() > 90) {
        for (x, what, target) in sprites::GENEX_COLOURS {
            let p = g.get_pixel(x, 0).0;
            rows.push(ColourRow {
                source: format!("GENEX x={x} {what}"),
                value: [p[0], p[1], p[2]],
                target,
            });
        }
    }
    (rows, unknown, font)
}

/// Reads a `.wsz` archive.
///
/// # Errors
/// What [`archive::unpack`] refuses, a refused bitmap, a sheet too small.
pub fn read(wsz: &[u8]) -> Result<Read, ImportError> {
    let (tree, ignored) = archive::unpack(wsz)?;
    let mut sheets = BTreeMap::new();
    for name in ["gen.bmp", "genex.bmp", "titlebar.bmp"] {
        if let Some(img) = sheet(&tree, name)? {
            sheets.insert(name, img);
        }
    }
    let mut decoded = Vec::new();
    let mut seen = Vec::new();
    for sprite in SPRITES {
        let img = sheets.get(sprite.sheet);
        if let Some(img) = img {
            decoded.push((sprite, sprites::crop(img, &sprite)?));
        }
        seen.push((sprite, img.is_some()));
    }
    let (colours, unknown, font) = colour_rows(&tree, sheets.get("genex.bmp"));
    let mut tokens = BTreeMap::new();
    for row in &colours {
        if let Target::Tokens(ids) = row.target {
            for id in ids {
                tokens.insert((*id).to_owned(), walk::color(row.value));
            }
        }
    }
    if let Some(face) = font.as_ref().and_then(|f| f.face) {
        tokens.insert(
            pledit::FONT_TOKEN.to_owned(),
            TokenValue::FontFamily(face.name.to_owned()),
        );
    }
    let has_gen = sheets.contains_key("gen.bmp");
    let ground = match tokens.get("bg.panel") {
        Some(TokenValue::Color { r, g, b, .. }) => [*r, *g, *b, 255],
        _ => [0; 4],
    };
    let frame = has_gen.then(|| {
        [
            sprites::frame(&decoded, "_SELECTED", ground),
            sprites::frame(&decoded, "", ground),
        ]
    });
    let title = has_gen.then(|| {
        [
            sprites::titlebar(&decoded, "_SELECTED"),
            sprites::titlebar(&decoded, ""),
        ]
    });
    if let Some(backing) = title.as_ref().and_then(|[a, _]| sprites::dominant(a)) {
        for id in ["titlebar.start", "titlebar.end"] {
            tokens.insert(id.to_owned(), walk::color(backing));
        }
    }
    Ok(Read {
        colours,
        tokens,
        sprites: seen,
        frame,
        title,
        controls: controls(&decoded),
        unknown,
        font,
        ignored,
    })
}

fn region(
    paths: [&str; 2],
    insets: rekindle_types::dto::skin::Insets,
    backing: Option<&str>,
) -> RegionSpec {
    RegionSpec {
        image: paths[0].to_owned(),
        fit: RegionFit::NineSlice(insets),
        states: BTreeMap::from([(RegionState::WindowInactive, paths[1].to_owned())]),
        tint: None,
        mask: None,
        text_backing: backing.map(str::to_owned),
    }
}

fn manifest(id: &str, read: &Read, source: &str, base: SkinBase) -> SkinManifest {
    let mut regions = BTreeMap::new();
    if read.frame.is_some() {
        regions.insert(
            "window.frame".to_owned(),
            region(FRAME, sprites::FRAME_INSETS, None),
        );
    }
    if read.title.is_some() {
        regions.insert(
            "titlebar".to_owned(),
            region(TITLE, sprites::TITLE_INSETS, Some("titlebar.start")),
        );
    }
    for c in &read.controls {
        let mut spec = c.spec();
        if c.region == sprites::BUTTON {
            spec.text_backing = Some(sprites::BUTTON_BACKING.to_owned());
        }
        regions.insert(c.region.to_owned(), spec);
    }
    SkinManifest {
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name: id.to_owned(),
        description: "Imported from a Winamp classic skin; see the NOTICE beside the package."
            .to_owned(),
        license: SkinLicense::Text(
            "Third-party Winamp skin art. Not covered by Rekindle's MIT licence; provenance and \
             every change are in the NOTICE beside the package."
                .to_owned(),
        ),
        source: Some(SkinSource {
            format: WINAMP_WSZ_FORMAT.to_owned(),
            hash: source.to_owned(),
        }),
        author: None,
        signature: None,
        variants: vec![SkinVariant {
            base,
            contrast: SkinContrast::Standard,
            tokens: TOKENS.to_owned(),
        }],
        regions,
        icons: BTreeMap::new(),
        sounds: BTreeMap::new(),
    }
}

fn png(file: &str, img: &RgbaImage) -> Result<Vec<u8>, ImportError> {
    encode_png(&DynamicImage::ImageRgba8(img.clone())).map_err(|e| ImportError::Image {
        file: file.to_owned(),
        detail: e.to_string(),
    })
}

/// Imports the `.wsz` bytes as the package `id`: a passing package, or a
/// draft naming every failing pair.
///
/// # Errors
/// What [`read`] refuses, an image that does not encode.
pub fn import(wsz: &[u8], id: &str) -> Result<Outcome, ImportError> {
    let read = read(wsz)?;
    let source = blake3::hash(wsz).to_hex().to_string();
    let (skin_base, base) = base_of(&read.tokens);
    let manifest_json =
        serde_json::to_vec(&manifest(id, &read, &source, skin_base)).map_err(|e| {
            ImportError::Xml {
                file: MANIFEST.to_owned(),
                detail: e.to_string(),
            }
        })?;
    let mut images = BTreeMap::new();
    for (paths, imgs) in [(FRAME, &read.frame), (TITLE, &read.title)] {
        if let Some(imgs) = imgs {
            for (p, img) in paths.iter().zip(imgs) {
                images.insert((*p).to_owned(), png(p, img)?);
            }
        }
    }
    for c in &read.controls {
        images.extend(c.files()?);
    }
    let build = |tokens: &BTreeMap<String, TokenValue>| {
        let mut files = images.clone();
        files.insert(MANIFEST.to_owned(), manifest_json.clone());
        files.insert(
            TOKENS.to_owned(),
            token_file(TOKENS, tokens).map_err(ImportError::Refused)?,
        );
        zip(&files).map_err(ImportError::Refused)
    };
    conclude(read.tokens.clone(), base, build, |changes| {
        notice::render(id, &read, &source, changes)
    })
}

#[cfg(test)]
mod tests;
