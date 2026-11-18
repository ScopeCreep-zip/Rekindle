//! `SkinList`'s entries in the golden DTO file.

use rekindle_types::dto::skin::{OriginalLicence, SkinCredits};
use rekindle_types::dto::skin_library::SkinEntry;
use serde_json::Value;

use super::{pk, to};

/// `SkinList`'s entries (plan E7.7.3d): a built-in with every credit, a
/// library skin stating no licence and naming no one.
pub(super) fn samples(add: &mut impl FnMut(&str, Value)) {
    add(
        "skin.entry.builtin",
        to(SkinEntry {
            id: "builtin:xfire-classic".into(),
            name: "Xfire".into(),
            imported_at: None,
            credits: Some(SkinCredits {
                authors: vec!["Xfire, Inc.".into()],
                contributors: vec!["Tim Brengle".into()],
                url: Some("http://www.xfire.com/".into()),
                licence: OriginalLicence::NoneStated,
            }),
        }),
    );
    add(
        "skin.entry.library",
        to(SkinEntry {
            id: format!("pkg:{}", pk('d')),
            name: "Night".into(),
            imported_at: Some(1_760_000_000_000),
            credits: Some(SkinCredits {
                authors: Vec::new(),
                contributors: Vec::new(),
                url: None,
                licence: OriginalLicence::Text("Free to share.".into()),
            }),
        }),
    );
    add(
        "skin.entry.uncredited",
        to(SkinEntry {
            id: format!("pkg:{}", pk('e')),
            name: "Plain".into(),
            imported_at: Some(1_760_000_000_001),
            credits: None,
        }),
    );
}
