//! Entry-name rules: one canonical, relative, forward-slash path inside the
//! package layout (Pidgin CVE-2014-3697 wrote outside its theme directory
//! through `..`, `\` and absolute names in an archive).

use rekindle_types::dto::skin::SkinRule;
use unicode_normalization::UnicodeNormalization;

/// The directories a package may hold; everything else sits at the root.
pub const LAYOUT_DIRS: [&str; 3] = ["assets", "sounds", "icons"];

/// The manifest's entry name.
pub const MANIFEST: &str = "manifest.json";

/// Checks a name's shape: no backslash, not absolute, no `..`, no empty or
/// `.` component, no control character. `name` has any trailing `/` of a
/// directory entry already removed.
pub(super) fn check_shape(name: &str) -> Result<(), SkinRule> {
    if name.contains('\\') {
        return Err(SkinRule::Backslash);
    }
    let bytes = name.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if name.starts_with('/') || drive {
        return Err(SkinRule::AbsolutePath);
    }
    for part in name.split('/') {
        if part == ".." {
            return Err(SkinRule::ParentTraversal);
        }
        if part.is_empty() || part == "." || part.chars().any(char::is_control) {
            return Err(SkinRule::NonCanonicalPath);
        }
    }
    Ok(())
}

/// Checks a name sits in the layout: a directory is one of
/// [`LAYOUT_DIRS`] or below one; a file is `manifest.json`, a root token
/// file (`*.json`) or below a layout directory.
pub(super) fn check_layout(name: &str, is_dir: bool) -> Result<(), SkinRule> {
    let in_layout = match name.split_once('/') {
        Some((top, _)) => LAYOUT_DIRS.contains(&top),
        None if is_dir => LAYOUT_DIRS.contains(&name),
        None => is_root_json(name),
    };
    if in_layout {
        Ok(())
    } else {
        Err(SkinRule::OutsideLayout)
    }
}

/// A root file name ending in `.json` (lowercase) with a non-empty stem:
/// the manifest or a token file.
pub fn is_root_json(name: &str) -> bool {
    !name.contains('/')
        && name
            .strip_suffix(".json")
            .is_some_and(|stem| !stem.is_empty())
}

/// The key two names collide under: NFC, then lowercase. Case-insensitive
/// and normalising file systems (APFS, NTFS) would make them one file.
pub(super) fn collision_key(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}
