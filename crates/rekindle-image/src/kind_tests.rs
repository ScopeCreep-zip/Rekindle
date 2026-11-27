use super::*;

#[test]
fn each_kind_is_known_by_its_magic() {
    assert_eq!(kind_of(b"\x89PNG\r\n\x1a\nrest"), Some(ImageKind::Png));
    assert_eq!(kind_of(b"\xff\xd8\xff\xe0rest"), Some(ImageKind::Jpeg));
    assert_eq!(kind_of(b"GIF87a...."), Some(ImageKind::Gif));
    assert_eq!(kind_of(b"GIF89a...."), Some(ImageKind::Gif));
    assert_eq!(kind_of(b"RIFF\0\0\0\0WEBPVP8L"), None);
    assert_eq!(kind_of(b"\x89PNG"), None, "a cut signature is no PNG");
    assert_eq!(kind_of(b""), None);
}
