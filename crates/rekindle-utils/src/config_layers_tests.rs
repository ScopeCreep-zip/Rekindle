use super::*;

fn table(text: &str) -> toml::Table {
    text.parse().unwrap()
}

#[test]
fn a_later_layer_can_restore_a_default() {
    let mut merged = table("[network]\nrpc_timeout_ms = 15000\ngossip_ttl = 7");
    merge(&mut merged, table("[network]\nrpc_timeout_ms = 8000"));
    assert_eq!(
        merged.to_string(),
        table("[network]\nrpc_timeout_ms = 8000\ngossip_ttl = 7").to_string()
    );
}

#[test]
fn nested_tables_merge_key_by_key() {
    let mut merged = table("[network.safety.voice]\nstability = \"reliable\"\nhop_count = 4");
    merge(
        &mut merged,
        table("[network.safety.voice]\nstability = \"low_latency\""),
    );
    let voice = &merged["network"]["safety"]["voice"];
    assert_eq!(voice["stability"].as_str(), Some("low_latency"));
    assert_eq!(voice["hop_count"].as_integer(), Some(4));
}

#[test]
fn drop_ins_apply_in_lexicographic_order_after_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let dropins = dir.path().join("config.d");
    std::fs::create_dir(&dropins).unwrap();
    std::fs::write(dir.path().join("config.toml"), "a = 1\nb = 1").unwrap();
    std::fs::write(dropins.join("20-late.toml"), "b = 3").unwrap();
    std::fs::write(dropins.join("10-early.toml"), "a = 2\nb = 2").unwrap();
    std::fs::write(dropins.join("ignored.txt"), "a = 9").unwrap();
    let loaded: toml::Table = load_layers(&[
        Layer::File(dir.path().join("config.toml")),
        Layer::DropIns(dropins),
    ])
    .unwrap();
    assert_eq!(loaded["a"].as_integer(), Some(2));
    assert_eq!(loaded["b"].as_integer(), Some(3));
}

#[test]
fn missing_optional_layers_are_skipped_but_explicit_ones_are_required() {
    let dir = tempfile::tempdir().unwrap();
    let none: toml::Table = load_layers(&[
        Layer::File(dir.path().join("absent.toml")),
        Layer::DropIns(dir.path().join("absent.d")),
    ])
    .unwrap();
    assert!(none.is_empty());
    assert!(matches!(
        load_layers::<toml::Table>(&[Layer::Required(dir.path().join("absent.toml"))]),
        Err(LayerError::Missing(_))
    ));
}

#[test]
fn a_broken_layer_names_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "network = [").unwrap();
    let err = load_layers::<toml::Table>(&[Layer::File(path.clone())]).unwrap_err();
    assert!(err.to_string().starts_with(&path.display().to_string()));
}
