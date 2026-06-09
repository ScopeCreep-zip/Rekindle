use std::path::Path;

fn v3_src_dir() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/src/v3")
}

fn scan_dir_for_pattern(dir: &Path, pattern: &str, violation_msg: &str) {
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .map_or(false, |ext| ext == "rs")
        })
    {
        let source = std::fs::read_to_string(entry.path())
            .expect("failed to read source file");
        assert!(
            !source.contains(pattern),
            "{violation_msg}\nFile: {}",
            entry.path().display()
        );
    }
}

#[test]
fn handlers_do_not_import_wire_types() {
    let handlers_dir = Path::new(v3_src_dir()).join("handlers");
    scan_dir_for_pattern(
        &handlers_dir,
        "v3::wire::",
        "Handler imports wire types directly. \
         Handlers must use codec domain types, not wire structs.",
    );
}

#[test]
fn handlers_do_not_use_from_le_bytes() {
    let handlers_dir = Path::new(v3_src_dir()).join("handlers");
    scan_dir_for_pattern(
        &handlers_dir,
        "from_le_bytes",
        "Handler performs byte manipulation. \
         Byte offsets must be in the codec layer only.",
    );
}

#[test]
fn handlers_do_not_use_to_le_bytes() {
    let handlers_dir = Path::new(v3_src_dir()).join("handlers");
    scan_dir_for_pattern(
        &handlers_dir,
        "to_le_bytes",
        "Handler performs byte serialization. \
         Byte offsets must be in the codec layer only.",
    );
}
