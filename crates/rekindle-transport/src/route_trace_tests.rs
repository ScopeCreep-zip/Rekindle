use rekindle_records::port::{PeerTarget, RouteClass};
use rekindle_utils::log_capture::CapturedLines;

fn capture(f: impl FnOnce()) -> String {
    let lines = CapturedLines::default();
    let sink = lines.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || sink.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    lines.text()
}

fn field<'a>(line: &'a str, name: &str) -> &'a str {
    let (_, rest) = line.split_once(&format!(" {name}=")).expect(name);
    rest.split(' ').next().expect(name)
}

const PEER: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";
const ROUTE: &str = "VLD0:um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";

#[test]
fn the_publish_and_import_lines_tag_one_blob_alike() {
    let blob = b"one route blob".to_vec();
    let other = b"a newer route blob".to_vec();
    let target = PeerTarget::new(ROUTE.to_owned());
    let out = capture(|| {
        super::published(RouteClass::General, Some(ROUTE), &blob, true);
        super::imported(PEER, &target, &blob);
        super::imported(PEER, &target, &other);
    });
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    let (published, imported, stale) = (lines[0], lines[1], lines[2]);
    assert!(
        published.contains("own route blob published"),
        "{published}"
    );
    assert!(
        imported.contains("peer route blob imported for a call"),
        "{imported}"
    );
    assert_eq!(field(published, "blob"), field(imported, "blob"), "{out}");
    assert_eq!(field(published, "route"), field(imported, "route"), "{out}");
    assert_ne!(field(imported, "blob"), field(stale, "blob"), "{out}");
    assert!(field(published, "blob").starts_with('<'), "{out}");
    for raw in [
        PEER,
        ROUTE.split_once(':').expect("typed").1,
        &rekindle_utils::blake3_hex(&blob),
    ] {
        assert!(!out.contains(raw), "`{raw}` reached the event: {out}");
    }
}
