//! Who made an imported Xfire skin, and so which licence text its manifest
//! and `NOTICE` carry (plan E7.7.3c).
//!
//! Xfire's own art is usable under the Rekindle owner's 2026-10-15 ruling
//! (Xfire is defunct). A skin proves that in one of two ways: its own
//! `Info.xml` names Xfire, Inc. as the original author (Symbiosis, Classic
//! v3), or a page outside the files says so, given to the importer as an
//! [`XfireCredit`] (Combat: Xfire's download page, `dl_1032.html:596`, "The
//! Combat skin is an official skin created by Xfire."; its zip holds no
//! author). Every other skin is third-party work published without a
//! licence. All three say Rekindle's MIT licence does not cover the pack.

use super::ImportError;
use crate::manifest::text::is_bidi_control;

/// The longest source URL, in scalars.
pub const MAX_SOURCE: usize = 512;
/// The longest quote, in scalars (a manifest description's bound).
pub const MAX_QUOTE: usize = 280;

/// Xfire's authorship stated outside the skin's files: the page and its
/// words, both shown in the `NOTICE` and the manifest licence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XfireCredit {
    source: String,
    quote: String,
}

fn bounded(field: &str, s: &str, max: usize) -> Result<(), ImportError> {
    let refuse = |detail: String| ImportError::Format {
        file: format!("--{field}"),
        detail,
    };
    if s.trim().is_empty() {
        return Err(refuse("empty".to_owned()));
    }
    let n = s.chars().count();
    if n > max {
        return Err(refuse(format!("{n} scalars, more than {max}")));
    }
    // Refused as in a manifest's text fields (Trojan Source, CVE-2021-42574).
    if let Some(c) = s.chars().find(|&c| c.is_control() || is_bidi_control(c)) {
        return Err(refuse(format!(
            "U+{:04X} is a control or bidi character",
            u32::from(c)
        )));
    }
    Ok(())
}

impl XfireCredit {
    /// A credit from a page's `http(s)://` URL and its words, which must
    /// name Xfire.
    ///
    /// # Errors
    /// [`ImportError::Format`] for an empty, overlong or non-URL source, an
    /// empty or overlong quote, a control or bidi character, or a quote that
    /// does not name Xfire.
    pub fn new(source: &str, quote: &str) -> Result<Self, ImportError> {
        bounded("credit-source", source, MAX_SOURCE)?;
        bounded("credit-quote", quote, MAX_QUOTE)?;
        if !(source.starts_with("https://") || source.starts_with("http://"))
            || source.contains(char::is_whitespace)
        {
            return Err(ImportError::Format {
                file: "--credit-source".to_owned(),
                detail: "not an http(s) URL".to_owned(),
            });
        }
        if !quote.contains("Xfire") {
            return Err(ImportError::Format {
                file: "--credit-quote".to_owned(),
                detail: "the quote does not name Xfire".to_owned(),
            });
        }
        Ok(Self {
            source: source.to_owned(),
            quote: quote.to_owned(),
        })
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn quote(&self) -> &str {
        &self.quote
    }
}

/// Which licence a pack carries.
#[derive(Debug, Clone, Copy)]
pub enum Authorship<'a> {
    /// The skin's own `Info.xml` names Xfire, Inc. as its original author.
    XfireByInfo,
    /// A page outside the files states it.
    XfireByCredit(&'a XfireCredit),
    /// Anyone else's work, published without a licence.
    ThirdParty,
}

const NOT_MIT: &str = "Not covered by Rekindle's MIT licence; provenance and every change are \
                       in the NOTICE beside the package.";

impl Authorship<'_> {
    /// The manifest's `license` text.
    #[must_use]
    pub fn manifest_text(self) -> String {
        match self {
            Self::XfireByInfo => format!(
                "Xfire, Inc.'s own skin art (Xfire is defunct), used under the Rekindle owner's \
                 2026-10-15 ruling. {NOT_MIT}"
            ),
            Self::XfireByCredit(c) => format!(
                "Xfire, Inc.'s own skin art, as {} states: \"{}\" (Xfire is defunct), used under \
                 the Rekindle owner's 2026-10-15 ruling. {NOT_MIT}",
                c.source, c.quote
            ),
            Self::ThirdParty => {
                format!("Third-party Xfire skin art, published without a licence. {NOT_MIT}")
            }
        }
    }

    /// The `NOTICE`'s licence paragraph, indented two spaces.
    #[must_use]
    pub fn notice_text(self) -> String {
        const RULING: &str = "Xfire is defunct; the Rekindle owner ruled on 2026-10-15 that\n  \
                              Xfire's own assets may be used, and replaces them with the same \
                              command.";
        let body = match self {
            Self::XfireByInfo => format!(
                "The skin art and palette are Xfire, Inc.'s own work (Info.xml's original\n  \
                 author). {RULING}"
            ),
            Self::XfireByCredit(c) => format!(
                "The skin art and palette are Xfire, Inc.'s own work, stated outside the\n  \
                 skin's files:\n    source: {}\n    quote: \"{}\"\n  {RULING}",
                c.source, c.quote
            ),
            Self::ThirdParty => {
                "The skin art and palette are third-party work published without a licence."
                    .to_owned()
            }
        };
        format!("Licence\n  {body}\n  Rekindle's MIT LICENSE does not cover this pack.\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://web.archive.org/web/2009/http://www.xfire.com/downloads/1032/";
    const QUOTE: &str = "The Combat skin is an official skin created by Xfire.";

    #[test]
    fn credits_are_bounded_and_checked() {
        let c = XfireCredit::new(PAGE, QUOTE).unwrap();
        assert_eq!((c.source(), c.quote()), (PAGE, QUOTE));
        assert!(XfireCredit::new("ftp://x/y", QUOTE).is_err());
        assert!(XfireCredit::new("https://a b", QUOTE).is_err());
        assert!(XfireCredit::new(PAGE, "").is_err());
        assert!(XfireCredit::new(PAGE, "An official skin.").is_err());
        assert!(XfireCredit::new(PAGE, "Xfire\u{202E} made it").is_err());
        assert!(XfireCredit::new(PAGE, "Xfire\nmade it").is_err());
        assert!(XfireCredit::new(PAGE, &format!("Xfire{}", "a".repeat(MAX_QUOTE))).is_err());
        let long = format!("https://{}", "a".repeat(MAX_SOURCE));
        assert!(XfireCredit::new(&long, QUOTE).is_err());
    }

    #[test]
    fn a_credit_reaches_both_texts() {
        let c = XfireCredit::new(PAGE, QUOTE).unwrap();
        let a = Authorship::XfireByCredit(&c);
        for text in [a.manifest_text(), a.notice_text()] {
            assert!(text.contains(PAGE) && text.contains(QUOTE), "{text}");
            assert!(text.contains("2026-10-15"), "{text}");
            assert!(!text.contains("Third-party"), "{text}");
        }
        assert!(Authorship::ThirdParty
            .manifest_text()
            .starts_with("Third-party"));
    }
}
