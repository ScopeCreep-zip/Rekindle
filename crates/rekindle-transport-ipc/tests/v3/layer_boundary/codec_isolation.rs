use std::path::Path;

fn v3_src_dir() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/src/v3")
}

#[test]
fn codec_does_not_import_handlers() {
    let codec_dir = Path::new(v3_src_dir()).join("codec");
    for entry in walkdir::WalkDir::new(&codec_dir)
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
            !source.contains("v3::handlers::"),
            "Codec imports handlers. Codec must not depend on handlers.\n\
             File: {}",
            entry.path().display()
        );
    }
}
