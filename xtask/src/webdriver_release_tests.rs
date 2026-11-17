use super::{
    feature_problems, graph_problems, has_feature_release_guard, workspace_names_driver, CRATE,
};

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
