//! The validator in its own process (plan E7.7.1 "Helper process"): the
//! real `rekindle-skin-helper` answers as the in-process pipeline does, and
//! a helper that hangs, dies or answers garbage is a typed failure.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};
use rekindle_skin::helper::{self, frame, HelperError, HelperOpts};
use rekindle_skin::limits::MAX_PACKAGE_BYTES;
use rekindle_skin::validate;

use crate::common::{self, Entry};

/// One token set; the rest take the dark defaults, which pass step 6.
const DARK: &[u8] =
    br#"{"size":{"chrome-height":{"$type":"dimension","$value":{"value":30,"unit":"px"}}}}"#;

fn real() -> HelperOpts {
    HelperOpts {
        path: PathBuf::from(env!("CARGO_BIN_EXE_rekindle-skin-helper")),
        timeout: helper::HELPER_TIMEOUT,
    }
}

fn valid() -> Vec<u8> {
    let icon = RgbaImage::from_pixel(3, 2, Rgba([90, 40, 90, 255]));
    let icon = encode_png(&DynamicImage::ImageRgba8(icon)).unwrap();
    common::zip(&[
        Entry::file("manifest.json", common::manifest().to_string().as_bytes()),
        Entry::file("dark.json", DARK),
        Entry::file("icons/c.png", &icon),
    ])
}

/// An executable shell script standing in for the helper.
#[cfg(unix)]
fn script(name: &str, body: &str) -> HelperOpts {
    use std::os::unix::fs::PermissionsExt;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("skin-helper-{name}"));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    HelperOpts {
        path,
        timeout: Duration::from_secs(10),
    }
}

#[test]
fn a_valid_package_round_trips_through_the_real_helper() {
    let bytes = valid();
    let in_process = validate::run(&bytes).expect("the package is valid");
    let out = helper::validate(&bytes, &real()).unwrap();
    assert_eq!(out, in_process);
    assert_eq!(out.hash, *blake3::hash(&out.bytes).as_bytes());
}

#[test]
fn a_refused_package_gives_the_in_process_refusal() {
    // `assets/frame.png` holds the three bytes "png": step 4 refuses it.
    let bytes = common::package(&common::manifest(), &[]);
    let in_process = validate::run(&bytes).unwrap_err();
    assert_eq!(
        helper::validate(&bytes, &real()),
        Err(HelperError::Refused(in_process))
    );
}

#[test]
fn an_oversized_package_is_refused_before_any_helper_starts() {
    let bytes = vec![0u8; usize::try_from(MAX_PACKAGE_BYTES).unwrap() + 1];
    let opts = HelperOpts {
        path: PathBuf::from("/nonexistent/rekindle-skin-helper"),
        timeout: Duration::from_secs(1),
    };
    let in_process = validate::run(&bytes).unwrap_err();
    assert_eq!(
        helper::validate(&bytes, &opts),
        Err(HelperError::Refused(in_process))
    );
}

#[cfg(unix)]
#[test]
fn a_helper_past_its_deadline_is_killed_and_reported() {
    let mut opts = script("sleep", "exec sleep 30");
    opts.timeout = Duration::from_millis(300);
    let started = Instant::now();
    let out = helper::validate(&valid(), &opts);
    assert_eq!(out, Err(HelperError::TimedOut(opts.timeout)));
    assert!(started.elapsed() < Duration::from_secs(10), "not killed");
}

#[cfg(unix)]
#[test]
fn a_helper_killed_by_a_signal_is_a_crash() {
    let opts = script("signal", "cat >/dev/null; kill -KILL $$");
    let out = helper::validate(&valid(), &opts);
    assert!(matches!(out, Err(HelperError::Crashed { .. })), "{out:?}");
}

#[cfg(unix)]
#[test]
fn a_helper_that_exits_non_zero_after_answering_is_a_crash() {
    // A whole refusal frame on stdout, then exit 101 as a panic does.
    let opts = script(
        "panic",
        "cat >/dev/null; printf 'RKSH\\001\\001\\002\\000\\000\\000{}'; exit 101",
    );
    let out = helper::validate(&valid(), &opts);
    assert!(matches!(out, Err(HelperError::Crashed { .. })), "{out:?}");
}

#[cfg(unix)]
#[test]
fn malformed_helper_output_is_refused() {
    for (name, out) in [
        ("garbage", "not a frame"),
        ("truncated", "RKSH\\001\\000\\377\\000\\000\\000abc"),
        ("empty", ""),
    ] {
        let opts = script(name, &format!("cat >/dev/null; printf '{out}'"));
        let got = helper::validate(&valid(), &opts);
        assert!(
            matches!(got, Err(HelperError::Malformed(_))),
            "{name}: {got:?}"
        );
    }
}

#[test]
fn the_frame_refuses_a_wrong_hash_or_trailing_bytes() {
    let canonical = validate::run(&valid()).unwrap();
    let good = frame::encode(&Ok(canonical.clone()));
    assert_eq!(frame::decode(&good), Ok(Ok(canonical)));
    let mut flipped = good.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 1;
    assert!(frame::decode(&flipped).is_err());
    let mut longer = good;
    longer.push(0);
    assert!(frame::decode(&longer).is_err());
}

#[test]
fn the_frame_refuses_another_version_or_trailing_bytes_after_a_refusal() {
    let refusal = validate::run(&common::package(&common::manifest(), &[])).unwrap_err();
    let good = frame::encode(&Err(refusal.clone()));
    assert_eq!(frame::decode(&good), Ok(Err(refusal)));
    let mut version = good.clone();
    version[4] = frame::VERSION + 1;
    assert!(frame::decode(&version).is_err());
    // JSON admits trailing white space: only the length refuses it.
    let mut longer = good;
    longer.push(b' ');
    assert!(frame::decode(&longer).is_err());
}

/// A built-in is validated by the real helper on first use and kept for
/// the process: a later call never starts the helper again.
#[test]
fn a_builtin_is_validated_once_per_process() {
    use rekindle_skin::builtin;
    let first = builtin::validated(builtin::DEFAULT, &real())
        .unwrap()
        .unwrap();
    let (_, pack) = builtin::find(builtin::DEFAULT).unwrap();
    assert_eq!(first.bytes, pack.bytes);
    let gone = HelperOpts {
        path: PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-such-skin-helper"),
        timeout: helper::HELPER_TIMEOUT,
    };
    assert!(matches!(
        helper::validate(pack.bytes, &gone),
        Err(HelperError::Spawn(_))
    ));
    assert_eq!(
        builtin::validated(builtin::DEFAULT, &gone).unwrap(),
        Some(first)
    );
    assert_eq!(builtin::validated("builtin:nope", &gone).unwrap(), None);
}
