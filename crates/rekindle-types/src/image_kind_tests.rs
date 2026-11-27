use super::ImageKind;

#[test]
fn the_wire_name_is_the_mime_type() {
    for kind in ImageKind::ALL {
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, format!("\"{}\"", kind.mime()));
        assert_eq!(serde_json::from_str::<ImageKind>(&json).unwrap(), kind);
    }
    assert!(serde_json::from_str::<ImageKind>("\"image/webp\"").is_err());
    assert!(serde_json::from_str::<ImageKind>("\"image/svg+xml\"").is_err());
}

#[test]
fn extensions() {
    assert_eq!(ImageKind::from_extension("PNG"), Some(ImageKind::Png));
    assert_eq!(ImageKind::from_extension("jpg"), Some(ImageKind::Jpeg));
    assert_eq!(ImageKind::from_extension("JPEG"), Some(ImageKind::Jpeg));
    assert_eq!(ImageKind::from_extension("gif"), Some(ImageKind::Gif));
    assert_eq!(ImageKind::from_extension("webp"), None);
    assert_eq!(ImageKind::from_extension(""), None);
}
