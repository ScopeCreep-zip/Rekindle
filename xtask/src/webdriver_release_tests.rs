use super::{
    feature_problems, graph_problems, has_feature_release_guard, has_release_guard,
    release_binary_problems, workspace_names_driver, ANSWER_DELAY_VAR, CRATE, EVAL_ROUTE,
    RELEASE_DIR_VAR,
};

#[test]
fn the_release_guard_is_recognised() {
    let guarded = "#[cfg(not(debug_assertions))]\ncompile_error!(\"no\");";
    assert!(has_release_guard(guarded));
    assert!(!has_release_guard("compile_error!(\"always\");"));
    assert!(!has_release_guard("#[cfg(test)]\ncompile_error!(\"no\");"));
}

#[test]
fn the_feature_release_guard_is_recognised() {
    let guarded =
        "#[cfg(all(feature = \"webdriver-test\", not(debug_assertions)))]\ncompile_error!(\"no\");";
    assert!(has_feature_release_guard(guarded));
    assert!(!has_feature_release_guard("compile_error!(\"always\");"));
    let debug_only = "#[cfg(not(debug_assertions))]\ncompile_error!(\"no\");";
    assert!(!has_feature_release_guard(debug_only));
    let other = "#[cfg(all(feature = \"other\", not(debug_assertions)))]\ncompile_error!(\"no\");";
    assert!(!has_feature_release_guard(other));
}

#[test]
fn a_workspace_dependency_on_the_driver_is_found_by_key_or_package() {
    let parse = |t: &str| t.parse::<toml_edit::DocumentMut>().unwrap();
    assert!(workspace_names_driver(&parse(&format!(
        "[workspace.dependencies]\n{CRATE} = {{ path = \"x\" }}"
    ))));
    assert!(workspace_names_driver(&parse(&format!(
        "[workspace.dependencies]\nd = {{ package = \"{CRATE}\", path = \"x\" }}"
    ))));
    assert!(!workspace_names_driver(&parse(
        "[workspace.dependencies]\nserde = \"1\""
    )));
}

#[test]
fn a_non_optional_dependency_is_a_problem() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let meta = crate::cargo_meta::Metadata::load(&root).unwrap();
    let parse = |t: &str| t.parse::<toml_edit::DocumentMut>().unwrap();
    let required = parse(&format!("[dependencies]\n{CRATE} = {{ path = \"x\" }}"));
    assert!(feature_problems(&meta, &required)
        .iter()
        .any(|p| p.contains("optional = true")));
}

/// The real workspace: nothing reaches the driver, and the GUI's build graph
/// is the one a release ships.
#[test]
fn nothing_in_the_workspace_reaches_the_driver() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let meta = crate::cargo_meta::Metadata::load(&root).unwrap();
    assert!(meta.build_graph("rekindle-desktop").contains("rekindle-db"));
    assert!(
        graph_problems(&meta).is_empty(),
        "{:?}",
        graph_problems(&meta)
    );
    let host = std::fs::read_to_string(root.join("src-tauri/Cargo.toml")).unwrap();
    let host = host.parse::<toml_edit::DocumentMut>().unwrap();
    let problems = feature_problems(&meta, &host);
    assert!(problems.is_empty(), "{problems:?}");
}

/// A release build of this gate holds the route as its needle and is not a
/// problem; a shipped binary holding it is.
#[test]
fn the_release_scan_skips_its_own_binary_only() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let meta = crate::cargo_meta::Metadata::load(&root).unwrap();
    let fake = std::env::temp_dir().join(format!("xtask-wdio-scan-{}", std::process::id()));
    let release = fake.join("target").join("release");
    std::fs::create_dir_all(&release).unwrap();
    let mut held = b"prefix".to_vec();
    held.extend_from_slice(EVAL_ROUTE);
    std::fs::write(release.join("xtask"), &held).unwrap();
    let own_only = release_binary_problems(&fake, &meta);
    std::fs::write(release.join("rekindle-desktop"), &held).unwrap();
    let shipped = release_binary_problems(&fake, &meta);
    std::fs::remove_dir_all(&fake).unwrap();
    assert!(own_only.is_empty(), "{own_only:?}");
    assert_eq!(
        shipped,
        vec!["target/release/rekindle-desktop holds /wdio/eval".to_owned()]
    );
}

/// A release binary naming the suite's held-answer switch is a problem, as
/// one holding the eval route is (each needle reported).
#[test]
fn the_release_scan_finds_the_held_answer_switch() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let meta = crate::cargo_meta::Metadata::load(&root).unwrap();
    let fake = std::env::temp_dir().join(format!("xtask-wdio-delay-{}", std::process::id()));
    let release = fake.join("target").join("release");
    std::fs::create_dir_all(&release).unwrap();
    let mut held = b"prefix".to_vec();
    held.extend_from_slice(ANSWER_DELAY_VAR);
    std::fs::write(release.join("rekindle-desktop"), &held).unwrap();
    let delay_only = release_binary_problems(&fake, &meta);
    held.extend_from_slice(EVAL_ROUTE);
    std::fs::write(release.join("rekindle-desktop"), &held).unwrap();
    let both = release_binary_problems(&fake, &meta);
    std::fs::remove_dir_all(&fake).unwrap();
    assert_eq!(
        delay_only,
        vec!["target/release/rekindle-desktop holds REKINDLE_WDIO_ANSWER_DELAY".to_owned()]
    );
    assert_eq!(
        both,
        vec![
            "target/release/rekindle-desktop holds /wdio/eval".to_owned(),
            "target/release/rekindle-desktop holds REKINDLE_WDIO_ANSWER_DELAY".to_owned(),
        ]
    );
}

/// A release binary naming the held-answer release directory is a problem.
#[test]
fn the_release_scan_finds_the_release_dir_switch() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let meta = crate::cargo_meta::Metadata::load(&root).unwrap();
    let fake = std::env::temp_dir().join(format!("xtask-wdio-release-{}", std::process::id()));
    let release = fake.join("target").join("release");
    std::fs::create_dir_all(&release).unwrap();
    let mut held = b"prefix".to_vec();
    held.extend_from_slice(RELEASE_DIR_VAR);
    std::fs::write(release.join("rekindle-desktop"), &held).unwrap();
    let found = release_binary_problems(&fake, &meta);
    std::fs::remove_dir_all(&fake).unwrap();
    assert_eq!(
        found,
        vec!["target/release/rekindle-desktop holds REKINDLE_WDIO_RELEASE_DIR".to_owned()]
    );
}
