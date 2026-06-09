use std::path::Path;

fn v3_src_dir() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/src/v3")
}

/// Dispatch may import wire enums (Lane, FrameClass, FrameKind variants)
/// because those ARE the dispatch keys. It must NOT import wire structs
/// (EnvelopeWire, StreamHeaderWire) — those belong to the codec layer.
#[test]
fn dispatch_does_not_import_wire_structs() {
    let dispatch_dir = Path::new(v3_src_dir()).join("dispatch");
    let forbidden = [
        "wire::envelope::EnvelopeWire",
        "wire::header::StreamHeaderWire",
        "wire::envelope::offsets",
        "wire::header::offsets",
        "from_le_bytes",
        "to_le_bytes",
    ];
    for entry in walkdir::WalkDir::new(&dispatch_dir)
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
        for pattern in &forbidden {
            assert!(
                !source.contains(pattern),
                "Dispatch imports wire struct or byte manipulation ({pattern}).\n\
                 Dispatch may use wire enums but not wire structs or byte offsets.\n\
                 File: {}",
                entry.path().display()
            );
        }
    }
}
