use super::*;

#[test]
fn the_bundle_floors_are_read_from_the_conf() {
    let ok = serde_json::json!({"bundle": {"macOS": {"minimumSystemVersion": "14.0"},
        "linux": {"deb": {"depends": [DEB_WEBKIT]}, "rpm": {"depends": [RPM_WEBKIT, "x"]}}}});
    assert!(conf_problems(&ok).is_empty());
    let old = serde_json::json!({"bundle": {"macOS": {"minimumSystemVersion": "10.15"},
        "linux": {"deb": {"depends": [DEB_WEBKIT]}}}});
    let hits = conf_problems(&old);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(
        hits[0].contains("10.15") && hits[1].contains("rpm"),
        "{hits:?}"
    );
}

#[test]
fn the_vite_targets_must_be_the_floor() {
    let ok =
        r#"target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome111" : "safari16.4","#;
    assert!(vite_problems(ok).is_empty());
    let low = ok.replace("chrome111", "chrome105");
    assert!(vite_problems(&low)[0].contains("chrome105"));
    assert_eq!(
        vite_targets(r#""windows" "esbuild" "es2022""#),
        vec!["es2022"]
    );
}

#[test]
fn the_release_doc_names_every_floor() {
    let doc = format!("{MACOS_FLOOR} {DEB_WEBKIT} {RPM_WEBKIT} chrome111 safari16.4");
    assert!(doc_problems(&doc).is_empty());
    assert_eq!(
        doc_problems(&doc.replace("chrome111", "chrome105")).len(),
        1
    );
}

#[test]
fn the_built_css_keeps_the_declared_layers_in_order() {
    let declared = declared_layers("/* a; */ @layer theme, base, a11y, sec;\n@import \"x\";");
    assert_eq!(declared, vec!["theme", "base", "a11y", "sec"]);
    let built = "@layer properties{a{--x:1}}@layer theme{:root{--c:red}}@layer base{b{margin:0}}@layer a11y{c{color:red}}@layer sec{d{color:red}}:root{--z:1}@media (x){@layer base{e{margin:0}}}";
    assert!(built_problems("o.css", built, &declared).is_empty());
    let swapped = built.replace(
        "@layer a11y{c{color:red}}@layer sec{d{color:red}}",
        "@layer sec{d{color:red}}@layer a11y{c{color:red}}",
    );
    assert!(built_problems("o.css", &swapped, &declared)[0].contains("opens the layers"));
    let dropped = built.replace("@layer sec{d{color:red}}", "");
    assert_eq!(built_problems("o.css", &dropped, &declared).len(), 1);
    let unlayered = format!("{built}.f{{color:red}}");
    assert!(built_problems("o.css", &unlayered, &declared)[0].contains("outside every layer"));
}
