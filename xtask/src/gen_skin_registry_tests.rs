use super::*;

#[test]
fn an_edited_generated_file_fails_until_regenerated() {
    let dir = std::env::temp_dir().join(format!("xtask-skin-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    gen_skin_registry(&dir).unwrap();
    check_skin_registry(&dir).unwrap();

    for (rel, _) in artefacts().unwrap() {
        let path = dir.join(&rel);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\n/* hand edit */\n");
        std::fs::write(&path, text).unwrap();
        let err = check_skin_registry(&dir).unwrap_err().to_string();
        assert!(err.contains(&format!("{rel}: stale")), "{err}");

        gen_skin_registry(&dir).unwrap();
        check_skin_registry(&dir).unwrap();
    }

    std::fs::remove_file(dir.join(&artefacts().unwrap()[0].0)).unwrap();
    assert!(check_skin_registry(&dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_artefact_carries_the_do_not_edit_header() {
    for (rel, content) in artefacts().unwrap().into_iter().filter(|(rel, _)| {
        Path::new(rel.as_str())
            .extension()
            .is_none_or(|e| e != "json")
    }) {
        let head = content.lines().next().unwrap_or_default();
        assert!(
            head.contains("cargo xtask gen-skin-registry") && head.contains("Do not edit"),
            "{rel}: {head}"
        );
    }
    let schema: serde_json::Value = serde_json::from_str(&json_schema()).unwrap();
    assert!(schema["$comment"]
        .as_str()
        .is_some_and(|c| c.contains("Do not edit")));
}

#[test]
fn the_committed_artefacts_are_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    check_skin_registry(root).unwrap();
}
