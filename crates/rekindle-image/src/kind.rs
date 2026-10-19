//! The magic bytes each [`ImageKind`] starts with. They only confirm the
//! kind a caller declared; nothing is dispatched on them.

use rekindle_types::image_kind::ImageKind;

/// PNG's eight-byte signature (PNG spec §5.2).
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
/// JPEG's SOI marker and the first byte of the next marker (ITU T.81 B.1.1).
const JPEG_MAGIC: &[u8] = b"\xff\xd8\xff";
/// The two GIF versions' signatures (GIF89a spec §17).
const GIF87A_MAGIC: &[u8] = b"GIF87a";
const GIF89A_MAGIC: &[u8] = b"GIF89a";

/// Whether `bytes` start with `kind`'s magic.
pub(crate) fn has_magic(bytes: &[u8], kind: ImageKind) -> bool {
    match kind {
        ImageKind::Png => bytes.starts_with(PNG_MAGIC),
        ImageKind::Jpeg => bytes.starts_with(JPEG_MAGIC),
        ImageKind::Gif => bytes.starts_with(GIF87A_MAGIC) || bytes.starts_with(GIF89A_MAGIC),
    }
}

/// The kind whose magic `bytes` start with, if any: what a mismatch error
/// reports. Never used to choose a decoder.
#[must_use]
pub fn kind_of(bytes: &[u8]) -> Option<ImageKind> {
    ImageKind::ALL
        .into_iter()
        .find(|kind| has_magic(bytes, *kind))
}

#[cfg(test)]
mod tests {
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
}
