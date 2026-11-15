//! Validator step 1: the outer size and the strict zip walk.

use rekindle_skin::limits::MAX_PACKAGE_BYTES;
use rekindle_skin::package;
use rekindle_types::dto::skin::{SkinError, SkinRule};

use crate::common::{self, manifest, Entry, Zip};

fn refused(bytes: &[u8]) -> SkinError {
    package::read(bytes).expect_err("the package must be refused")
}

/// The minimal package with `extra` entries added, refused.
fn refused_with(extra: &[Entry]) -> SkinError {
    refused(&common::package(&manifest(), extra))
}

fn assert_rule(e: &SkinError, entry: &str, rule: &SkinRule) {
    assert_eq!((e.entry.as_str(), &e.rule), (entry, rule), "{e}");
}

fn assert_malformed(e: &SkinError, needle: &str) {
    match &e.rule {
        SkinRule::MalformedZip { detail } => assert!(detail.contains(needle), "{e}"),
        other => panic!("expected MalformedZip({needle}), got {other:?}"),
    }
}

#[test]
fn the_minimal_package_is_read() {
    let bytes = common::package(&manifest(), &[]);
    let p = package::read(&bytes).unwrap();
    assert_eq!(p.get("dark.json"), Some(&b"{}"[..]));
    assert_eq!(p.get("assets/frame.png"), Some(&b"png"[..]));
    assert_eq!(p.get("assets/"), None, "directories are dropped");
}

#[test]
fn a_package_over_16_mib_is_refused_before_parsing() {
    let bytes = vec![0u8; usize::try_from(MAX_PACKAGE_BYTES).unwrap() + 1];
    let size = MAX_PACKAGE_BYTES + 1;
    let max = MAX_PACKAGE_BYTES;
    assert_rule(
        &refused(&bytes),
        SkinError::PACKAGE,
        &SkinRule::PackageTooLarge { size, max },
    );
}

#[test]
fn bytes_that_are_not_a_zip_are_refused() {
    assert_malformed(
        &refused(b"PK\x03\x04 not really"),
        "end of central directory",
    );
}

#[test]
fn more_than_512_entries_are_refused() {
    let extra: Vec<Entry> = (0..509)
        .map(|i| Entry::file(&format!("assets/{i}.png"), b"x"))
        .collect();
    let rule = SkinRule::TooManyEntries {
        count: 513,
        max: 512,
    };
    assert_rule(&refused_with(&extra), SkinError::PACKAGE, &rule);
}

#[test]
fn a_deflated_entry_is_refused() {
    // Raw DEFLATE of "aaaa" (method 8); the walk must never inflate it.
    let e = Entry {
        method: 8,
        ..Entry::file("assets/a.png", &[0x4b, 0x4c, 0x04, 0x02, 0x00])
    };
    assert_rule(
        &refused_with(&[e]),
        "assets/a.png",
        &SkinRule::CompressedEntry { method: 8 },
    );
}

#[test]
fn an_encrypted_entry_is_refused() {
    let e = Entry {
        flags: 1,
        ..Entry::file("assets/a.png", b"x")
    };
    assert_rule(
        &refused_with(&[e]),
        "assets/a.png",
        &SkinRule::EncryptedEntry,
    );
}

#[test]
fn a_unix_symlink_is_refused() {
    let e = Entry {
        external_attrs: 0o120_777 << 16,
        ..Entry::file("assets/a.png", b"/etc/passwd")
    };
    assert_rule(&refused_with(&[e]), "assets/a.png", &SkinRule::Symlink);
}

#[test]
fn a_windows_reparse_point_is_refused() {
    let e = Entry {
        made_by: 0x0014,
        external_attrs: 0x420,
        ..Entry::file("assets/a.png", b"x")
    };
    assert_rule(&refused_with(&[e]), "assets/a.png", &SkinRule::Symlink);
}

#[test]
fn a_fifo_is_refused() {
    let e = Entry {
        external_attrs: 0o010_644 << 16,
        ..Entry::file("assets/a.png", b"")
    };
    let rule = SkinRule::SpecialFile { mode: 0o010_644 };
    assert_rule(&refused_with(&[e]), "assets/a.png", &rule);
}

/// Pidgin CVE-2014-3697: archive names that escape the theme directory.
#[test]
fn cve_2014_3697_paths_are_refused() {
    let cases = [
        ("../evil.json", SkinRule::ParentTraversal),
        ("assets/../../evil.png", SkinRule::ParentTraversal),
        ("assets/..", SkinRule::ParentTraversal),
        ("..\\evil.json", SkinRule::Backslash),
        ("assets\\..\\..\\evil.png", SkinRule::Backslash),
        ("\\\\server\\share\\evil.png", SkinRule::Backslash),
        ("/etc/passwd", SkinRule::AbsolutePath),
        ("C:/Windows/evil.png", SkinRule::AbsolutePath),
        ("C:evil.json", SkinRule::AbsolutePath),
    ];
    for (name, rule) in cases {
        assert_rule(&refused_with(&[Entry::file(name, b"x")]), name, &rule);
    }
}

#[test]
fn empty_and_dot_components_are_refused() {
    for name in ["assets//a.png", "assets/./a.png", "assets/a\u{7}.png"] {
        assert_rule(
            &refused_with(&[Entry::file(name, b"x")]),
            name,
            &SkinRule::NonCanonicalPath,
        );
    }
}

#[test]
fn entries_outside_the_layout_are_refused() {
    for name in ["evil.exe", "scripts/run.js", ".json", "themes/"] {
        let e = if name.ends_with('/') {
            Entry::dir(name)
        } else {
            Entry::file(name, b"x")
        };
        assert_rule(&refused_with(&[e]), name, &SkinRule::OutsideLayout);
    }
}

#[test]
fn a_duplicate_name_is_refused() {
    let e = Entry::file("assets/frame.png", b"second");
    assert_rule(
        &refused_with(&[e]),
        "assets/frame.png",
        &SkinRule::DuplicateName,
    );
}

#[test]
fn a_case_colliding_name_is_refused() {
    let rule = SkinRule::CaseCollision {
        other: "assets/frame.png".into(),
    };
    assert_rule(
        &refused_with(&[Entry::file("assets/Frame.PNG", b"x")]),
        "assets/Frame.PNG",
        &rule,
    );
}

#[test]
fn an_nfc_colliding_name_is_refused() {
    // "é" precomposed, then "e" + U+0301: one file on APFS.
    let composed = Entry::file("assets/caf\u{e9}.png", b"x");
    let decomposed = Entry::file("assets/cafe\u{301}.png", b"y");
    let rule = SkinRule::CaseCollision {
        other: "assets/caf\u{e9}.png".into(),
    };
    assert_rule(
        &refused_with(&[composed, decomposed]),
        "assets/cafe\u{301}.png",
        &rule,
    );
}

#[test]
fn a_crc_mismatch_is_refused() {
    let e = Entry {
        crc: Some(0xDEAD_BEEF),
        ..Entry::file("assets/a.png", b"x")
    };
    assert_rule(&refused_with(&[e]), "assets/a.png", &SkinRule::CrcMismatch);
}

#[test]
fn a_package_without_a_manifest_is_refused() {
    let bytes = common::zip(&[Entry::file("dark.json", b"{}")]);
    assert_rule(
        &refused(&bytes),
        "manifest.json",
        &SkinRule::MissingManifest,
    );
}

#[test]
fn a_non_ascii_name_without_the_utf8_flag_is_refused() {
    let e = Entry {
        flags: 0,
        ..Entry::file("assets/\u{e9}.png", b"x")
    };
    assert_malformed(&refused_with(&[e]), "UTF-8 flag");
}

#[test]
fn a_local_header_that_disagrees_with_the_directory_is_refused() {
    let mut z = Zip::default();
    let offset = z.add(&Entry::file("manifest.json", b"{}"));
    z.add_central(&Entry::file("dark.json", b"{}"), offset);
    assert_malformed(&refused(&z.finish()), "disagrees");
}

/// Entry B's local header hides inside entry A's data: two names, one
/// span of bytes (the overlapping-file zip bomb shape).
#[test]
fn overlapping_entries_are_refused() {
    let hidden = Entry::file("assets/b.png", b"b");
    let outer = Entry::file("assets/a.png", &hidden.local());
    let mut z = Zip::default();
    z.add(&Entry::file("manifest.json", b"{}"));
    let a = z.add(&outer);
    z.add_central(&hidden, a + 30 + outer.name.len());
    assert_malformed(&refused(&z.finish()), "overlaps");
}

#[test]
fn trailing_bytes_after_the_end_record_are_refused() {
    let mut bytes = common::package(&manifest(), &[]);
    bytes.push(0);
    assert_malformed(&refused(&bytes), "end of central directory");
}

#[test]
fn a_non_unix_entry_with_plain_attributes_is_read() {
    let e = Entry {
        made_by: 0x0014,
        external_attrs: 0x20,
        ..Entry::file("icons/a.png", b"x")
    };
    let bytes = common::package(&manifest(), &[e]);
    assert!(package::read(&bytes).unwrap().get("icons/a.png").is_some());
}
