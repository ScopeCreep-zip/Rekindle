//! The kind of an image, declared by whoever hands it over (plan E7.11):
//! the file picker's MIME type for an upload, a file's extension on the
//! command line, the manifest for a skin. `rekindle-image` decodes by this
//! kind alone and refuses bytes of another kind; it never guesses one.

use serde::{Deserialize, Serialize};

/// An image kind Rekindle decodes. On the wire (JSON) it is the MIME type
/// a browser reports for the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageKind {
    /// PNG.
    #[serde(rename = "image/png")]
    Png,
    /// JPEG.
    #[serde(rename = "image/jpeg")]
    Jpeg,
    /// GIF, still or animated.
    #[serde(rename = "image/gif")]
    Gif,
}

impl ImageKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 3] = [Self::Png, Self::Jpeg, Self::Gif];

    /// The MIME type, as a browser reports it.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
        }
    }

    /// The kind a file name's extension declares (`png`, `jpg`, `jpeg`,
    /// `gif`, any case), or `None`.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            _ => None,
        }
    }
}

impl std::fmt::Display for ImageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.mime())
    }
}

#[cfg(test)]
#[path = "image_kind_tests.rs"]
mod tests;
