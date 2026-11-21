//! A draft's assets (plan E7.7.5): what "Start from the current skin"
//! carries from the source package (its regions, icons, sounds and
//! window trees, with every entry they name) and the regions the author imported, a PNG or
//! GIF each with its fit. Nothing here checks an image: the exported
//! package goes through the whole validator, so an imported region is
//! held to the same rules as any package's (a bad nine-slice is refused
//! by name, [`crate::regions::validate`]).

use std::collections::BTreeMap;

use rekindle_types::dto::skin::{RegionSpec, SkinError, SkinRule, SkinWindow, WindowLayout};
use rekindle_types::dto::skin_draft::SkinDraft;
use rekindle_types::dto::skin_tree::SkinNode;

use crate::{manifest, package};

/// The bytes a draft's export needs that the draft itself does not hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assets {
    /// The canonical package the draft started from (`SkinDraft::from`).
    pub source: Option<Vec<u8>>,
    /// Each imported region's image, by region id, read from its path.
    pub regions: BTreeMap<String, Vec<u8>>,
}

/// The manifest maps the export carries.
#[derive(Debug, Default)]
pub(super) struct Carried {
    pub regions: BTreeMap<String, RegionSpec>,
    pub icons: BTreeMap<String, String>,
    pub sounds: BTreeMap<String, String>,
    /// The frame layouts (plan SK0 S1), carried whole with their tiles.
    pub windows: BTreeMap<SkinWindow, WindowLayout>,
}

/// The entry an imported region's image takes: its type from its magic
/// bytes (the validator decodes it as that type and refuses a mismatch).
fn entry_of(region: &str, bytes: &[u8]) -> String {
    let ext = if bytes.starts_with(b"GIF8") {
        "gif"
    } else {
        "png"
    };
    format!("assets/regions/{region}.{ext}")
}

/// Every entry a component tree's nodes name (plan SK2: rule 9, a node's
/// art and its states are in the package).
fn tree_art<'a>(nodes: &'a [SkinNode], out: &mut Vec<&'a String>) {
    for n in nodes {
        if let Some(a) = &n.art {
            out.push(&a.image);
            out.extend(a.states.values());
        }
        tree_art(&n.children, out);
    }
}

fn names(spec: &RegionSpec) -> impl Iterator<Item = &String> {
    std::iter::once(&spec.image)
        .chain(spec.states.values())
        .chain(spec.mask.as_ref())
}

/// The source's maps and entries, then each imported region in place of
/// the source's (an entry only the replaced region named is dropped).
///
/// # Errors
/// A source that does not read, or an imported region without its bytes.
pub(super) fn gather(
    draft: &SkinDraft,
    assets: &Assets,
) -> Result<(Carried, BTreeMap<String, Vec<u8>>), SkinError> {
    let mut carried = Carried::default();
    let mut files = BTreeMap::new();
    if let Some(bytes) = &assets.source {
        let p = package::read(bytes)?;
        let m = manifest::parse(&p)?;
        let named = m
            .regions
            .values()
            .flat_map(names)
            .chain(m.icons.values())
            .chain(m.sounds.values())
            .chain(
                m.windows
                    .values()
                    .flat_map(|w| w.tiles.iter().map(|t| &t.image)),
            )
            .chain(m.windows.values().flat_map(|w| {
                w.title.iter().flat_map(|t| &t.parts).flat_map(|p| {
                    p.image
                        .iter()
                        .chain(p.states.values())
                        .chain(p.slices.iter().flat_map(|s| [&s.left, &s.middle, &s.right]))
                })
            }));
        let mut in_trees = Vec::new();
        for w in m.windows.values() {
            if let Some(t) = &w.tree {
                tree_art(&t.root, &mut in_trees);
            }
        }
        for name in named.chain(in_trees) {
            if let Some(data) = p.get(name) {
                files.insert(name.clone(), data.to_vec());
            }
        }
        carried = Carried {
            regions: m.regions,
            icons: m.icons,
            sounds: m.sounds,
            windows: m.windows,
        };
    }
    for (id, region) in &draft.regions {
        let bytes = assets.regions.get(id).ok_or_else(|| {
            SkinError::new(
                region.path.as_str(),
                SkinRule::MissingEntry {
                    field: format!("regions.{id}"),
                },
            )
        })?;
        if let Some(old) = carried.regions.remove(id) {
            let still: Vec<&String> = carried
                .regions
                .values()
                .flat_map(names)
                .chain(carried.icons.values())
                .collect();
            for name in names(&old).filter(|n| !still.contains(n)) {
                files.remove(name);
            }
        }
        let image = entry_of(id, bytes);
        files.insert(image.clone(), bytes.clone());
        carried.regions.insert(
            id.clone(),
            RegionSpec {
                image,
                fit: region.fit,
                states: BTreeMap::new(),
                tint: None,
                mask: None,
                text_backing: region.text_backing.clone(),
            },
        );
    }
    Ok((carried, files))
}
