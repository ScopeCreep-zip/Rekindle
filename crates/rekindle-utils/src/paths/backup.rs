//! Keeping `state/` out of OS backups (plan E7.1, audit 2026-10-20 R3).
//!
//! `state/` holds every identity's database (the Double Ratchet sessions,
//! the prekey secrets), its vault and Veilid's storage. A restored copy of
//! a ratchet re-uses message keys and brings consumed one-time prekeys back
//! (threat model I14), so the directory is marked for exclusion by the
//! backup tools that honour a per-directory mark:
//!
//! - macOS, Time Machine: the sticky exclusion `NSURLIsExcludedFromBackupKey`
//!   and `tmutil addexclusion` set, the extended attribute
//!   `com.apple.metadata:com_apple_backup_excludeItem` holding a binary
//!   plist of the string `com.apple.backupd` (read back from
//!   `tmutil addexclusion` on macOS 27.2; `tmutil isexcluded` reports the
//!   directory `[Excluded]`). It is written with rustix's safe `setxattr`,
//!   so no Objective-C call and no `unsafe` block is needed. The mark sits
//!   on the directory, so it covers every file in it.
//! - Linux, Déjà Dup (GNOME's backup tool): "Any folder with a CACHEDIR.TAG
//!   or .deja-dup-ignore file in it" is ignored by default
//!   (help.gnome.org/deja-dup/prefs.html). `.deja-dup-ignore` is used:
//!   `CACHEDIR.TAG` declares the folder a regenerable cache, which cleaners
//!   may delete, and this folder is the user's identity.
//! - Windows has no per-user mark: the VSS backup and snapshot exclusions
//!   (`FilesNotToBackup`, `FilesNotToSnapshot`) are `HKLM` keys that need
//!   elevation, so they belong to the installer (plan F3's NSIS hook).

use std::path::Path;

use super::DataRoot;

/// Time Machine's sticky-exclusion attribute.
#[cfg(target_os = "macos")]
pub const TIME_MACHINE_XATTR: &str = "com.apple.metadata:com_apple_backup_excludeItem";

/// The attribute's value: `bplist00` holding the string `com.apple.backupd`,
/// byte for byte what `tmutil addexclusion` writes.
#[cfg(target_os = "macos")]
pub const TIME_MACHINE_XATTR_VALUE: &[u8] = &[
    0x62, 0x70, 0x6C, 0x69, 0x73, 0x74, 0x30, 0x30, 0x5F, 0x10, 0x11, 0x63, 0x6F, 0x6D, 0x2E, 0x61,
    0x70, 0x70, 0x6C, 0x65, 0x2E, 0x62, 0x61, 0x63, 0x6B, 0x75, 0x70, 0x64, 0x08, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1C,
];

/// Déjà Dup's ignore marker, an empty file in the excluded folder.
#[cfg(target_os = "linux")]
pub const DEJA_DUP_MARKER: &str = ".deja-dup-ignore";

impl DataRoot {
    /// Mark `state/` excluded from OS backups (see the module docs). Run
    /// after the layout check settled `state/` (`rekindle_node::host::claim`),
    /// since a layout reset empties the directory.
    ///
    /// # Errors
    /// The mark cannot be written (a volume without extended attributes,
    /// a read-only directory).
    pub fn exclude_state_from_backup(&self) -> std::io::Result<()> {
        exclude_from_backup(&self.state)
    }
}

/// Mark `dir` excluded from OS backups.
#[cfg(target_os = "macos")]
fn exclude_from_backup(dir: &Path) -> std::io::Result<()> {
    rustix::fs::setxattr(
        dir,
        TIME_MACHINE_XATTR,
        TIME_MACHINE_XATTR_VALUE,
        rustix::fs::XattrFlags::empty(),
    )
    .map_err(std::io::Error::from)
}

/// Mark `dir` excluded from OS backups.
#[cfg(target_os = "linux")]
fn exclude_from_backup(dir: &Path) -> std::io::Result<()> {
    std::fs::write(dir.join(DEJA_DUP_MARKER), [])
}

/// Mark `dir` excluded from OS backups: nothing to write per user on this
/// platform (the module docs).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn exclude_from_backup(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tmutil isexcluded`'s verdict on `path`.
    #[cfg(target_os = "macos")]
    fn time_machine_says(path: &Path) -> String {
        let out = std::process::Command::new("tmutil")
            .arg("isexcluded")
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn time_machine_reports_state_excluded() {
        // The system temp directory and cargo's `target/` are excluded
        // already, so the root is made in the crate's own directory.
        let base = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = DataRoot::under(base.path());
        root.create_dirs().unwrap();
        assert!(time_machine_says(&root.state).starts_with("[Included]"));
        root.exclude_state_from_backup().unwrap();
        let said = time_machine_says(&root.state);
        assert!(said.starts_with("[Excluded]"), "{said}");
        // A file in it is excluded with it; only `state/` is marked.
        let db = root.identity_db("pk");
        std::fs::write(&db, b"x").unwrap();
        assert!(time_machine_says(&db).starts_with("[Excluded]"));
        assert!(time_machine_says(&root.config).starts_with("[Included]"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn deja_dup_marker_is_in_state() {
        let base = tempfile::tempdir().unwrap();
        let root = DataRoot::under(base.path());
        root.create_dirs().unwrap();
        root.exclude_state_from_backup().unwrap();
        assert!(root.state.join(DEJA_DUP_MARKER).is_file());
        assert!(!root.config.join(DEJA_DUP_MARKER).exists());
    }

    #[test]
    fn marking_twice_is_harmless() {
        let base = tempfile::tempdir().unwrap();
        let root = DataRoot::under(base.path());
        root.create_dirs().unwrap();
        root.exclude_state_from_backup().unwrap();
        root.exclude_state_from_backup().unwrap();
    }
}
