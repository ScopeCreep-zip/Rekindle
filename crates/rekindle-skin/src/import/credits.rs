//! The human credits an Xfire skin states about itself (plan E7.7.3d): its
//! original authors and contributors, where it was published, and the
//! licence text its own files carry, or that they carry none. They are
//! recorded as stated, never verified; the manifest validator checks them
//! like `name` (no control or bidi characters) and refuses the package
//! otherwise.

use rekindle_types::dto::skin::{OriginalLicence, SkinCredits};
use unicode_normalization::UnicodeNormalization;

use super::credit::XfireCredit;
use super::Tree;
use crate::manifest::MAX_CREDIT_NAMES;

/// The author `Info.xml` or an [`XfireCredit`] names on Xfire's own skins.
pub const XFIRE_AUTHOR: &str = "Xfire, Inc.";
/// A credited name's bound, in scalars (the manifest `name`'s).
const MAX_NAME: usize = 64;
/// A licence text's bound, in scalars: a quarter of the manifest's own
/// size bound, so the manifest holding it stays inside it.
const MAX_LICENCE_SCALARS: usize = 16 * 1024;

/// The file stems read as the skin's licence, in the order tried: a
/// licence file before a README (fan skins state their terms in one).
const LICENCE_STEMS: [&str; 3] = ["license", "licence", "readme"];

/// Windows-1252's 0x80–0x9F (the bytes where it differs from Latin-1);
/// `None` for the five it leaves undefined.
const CP1252_HIGH: [Option<char>; 32] = [
    Some('€'),
    None,
    Some('‚'),
    Some('ƒ'),
    Some('„'),
    Some('…'),
    Some('†'),
    Some('‡'),
    Some('ˆ'),
    Some('‰'),
    Some('Š'),
    Some('‹'),
    Some('Œ'),
    None,
    Some('Ž'),
    None,
    None,
    Some('‘'),
    Some('’'),
    Some('“'),
    Some('”'),
    Some('•'),
    Some('–'),
    Some('—'),
    Some('˜'),
    Some('™'),
    Some('š'),
    Some('›'),
    Some('œ'),
    None,
    Some('ž'),
    Some('Ÿ'),
];

/// One credited name: trimmed and cut to the manifest's bound, as the
/// importers cut `name`; empty names are not credits.
fn name(raw: &str) -> Option<String> {
    let n: String = raw.trim().chars().take(MAX_NAME).collect();
    (!n.is_empty()).then_some(n)
}

/// Names in order, each once, at most [`MAX_CREDIT_NAMES`], none of
/// `except`.
pub(crate) fn names<'a>(raw: impl IntoIterator<Item = &'a str>, except: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in raw.into_iter().filter_map(name) {
        if out.len() < MAX_CREDIT_NAMES && !out.contains(&n) && !except.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// A URL the skin states, kept only when it is an `http(s)` URL the
/// manifest accepts; anything else is not a source link.
pub(crate) fn url(raw: &str) -> Option<String> {
    let u = raw.trim();
    let ok = (u.starts_with("https://") || u.starts_with("http://"))
        && u.bytes().all(|b| b.is_ascii_graphic())
        && u.len() <= crate::manifest::MAX_CREDIT_URL;
    ok.then(|| u.to_owned())
}

/// UTF-8 (a BOM stripped), else Windows-1252 (the Xfire era's encoding),
/// whose undefined bytes stay as U+FFFD for the reader to see.
fn decode(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.strip_prefix('\u{FEFF}').unwrap_or(s).to_owned();
    }
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)].unwrap_or('\u{FFFD}'),
            _ => char::from(b),
        })
        .collect()
}

/// The skin's licence: the first root `license*.txt`, `licence*.txt` or
/// `readme*.txt` (ASCII case-insensitive), as text with Windows line ends
/// made line feeds and tabs spaces, NFC; otherwise "no licence stated". A
/// text over 16,384 scalars is cut at the bound. Any other
/// control or bidi character stays, and the validator refuses it.
#[must_use]
pub fn licence(tree: &Tree) -> OriginalLicence {
    let file = LICENCE_STEMS.iter().find_map(|stem| {
        tree.iter().find(|(path, _)| {
            let p = path.to_ascii_lowercase();
            !p.contains('/')
                && p.starts_with(stem)
                && std::path::Path::new(&p)
                    .extension()
                    .is_some_and(|e| e == "txt")
        })
    });
    let Some((_, bytes)) = file else {
        return OriginalLicence::NoneStated;
    };
    let text = decode(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
        .nfc()
        .take(MAX_LICENCE_SCALARS)
        .collect::<String>();
    let text = text.trim_matches('\n');
    if text.trim().is_empty() {
        OriginalLicence::NoneStated
    } else {
        OriginalLicence::Text(text.to_owned())
    }
}

/// Credits from what the skin states (`authors`, `contributors`, `url`)
/// and an outside [`XfireCredit`]: the credit adds Xfire, Inc. as an
/// original author and is the source link when the skin names none.
#[must_use]
pub fn build(
    tree: &Tree,
    authors: &[String],
    contributors: &[String],
    stated_url: Option<&str>,
    credit: Option<&XfireCredit>,
) -> SkinCredits {
    let by_credit = credit.map(|_| XFIRE_AUTHOR);
    let authors = names(authors.iter().map(String::as_str).chain(by_credit), &[]);
    let contributors = names(contributors.iter().map(String::as_str), &authors);
    SkinCredits {
        url: stated_url
            .and_then(url)
            .or_else(|| credit.and_then(|c| url(c.source()))),
        contributors,
        authors,
        licence: licence(tree),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &[u8])]) -> Tree {
        files
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.to_vec()))
            .collect()
    }

    #[test]
    fn names_are_trimmed_bounded_and_unique() {
        let long = "a".repeat(70);
        let raw = [" Ann ", "", "Ann", long.as_str(), "Bo"];
        let got = names(raw, &["Bo".to_owned()]);
        assert_eq!(got, vec!["Ann".to_owned(), "a".repeat(64)]);
        let many: Vec<String> = (0..20).map(|i| format!("n{i}")).collect();
        assert_eq!(
            names(many.iter().map(String::as_str), &[]).len(),
            MAX_CREDIT_NAMES
        );
    }

    #[test]
    fn only_http_urls_are_source_links() {
        assert_eq!(
            url(" http://www.xfire.com/ ").as_deref(),
            Some("http://www.xfire.com/")
        );
        assert_eq!(url("www.xfire.com"), None);
        assert_eq!(url("javascript:alert(1)"), None);
        assert_eq!(url("https://a b"), None);
    }

    #[test]
    fn a_licence_file_is_read_before_a_readme() {
        assert_eq!(
            licence(&tree(&[("a.ini", b"x")])),
            OriginalLicence::NoneStated
        );
        let t = tree(&[
            ("ReadMe.txt", b"By Ann.\r\nAll rights reserved.\r\n"),
            ("sub/license.txt", b"not the root"),
        ]);
        assert_eq!(
            licence(&t),
            OriginalLicence::Text("By Ann.\nAll rights reserved.".into())
        );
        let t = tree(&[
            ("LICENSE.TXT", b"Free\tto share \x93as is\x94."),
            ("readme.txt", b"r"),
        ]);
        assert_eq!(
            licence(&t),
            OriginalLicence::Text("Free    to share \u{201C}as is\u{201D}.".into())
        );
        assert_eq!(
            licence(&tree(&[("readme.txt", b"\r\n ")])),
            OriginalLicence::NoneStated
        );
    }

    #[test]
    fn a_credit_names_xfire_and_is_the_link_when_the_skin_names_none() {
        let c = XfireCredit::new("https://example.org/combat", "Xfire made Combat.").unwrap();
        let t = tree(&[]);
        let got = build(&t, &[], &[], None, Some(&c));
        assert_eq!(got.authors, vec![XFIRE_AUTHOR.to_owned()]);
        assert_eq!(got.url.as_deref(), Some("https://example.org/combat"));
        assert_eq!(got.licence, OriginalLicence::NoneStated);
        let stated = build(
            &t,
            &[XFIRE_AUTHOR.to_owned()],
            &[XFIRE_AUTHOR.to_owned(), "Tim Brengle".to_owned()],
            Some("http://www.xfire.com/"),
            Some(&c),
        );
        assert_eq!(stated.authors, vec![XFIRE_AUTHOR.to_owned()]);
        assert_eq!(stated.contributors, vec!["Tim Brengle".to_owned()]);
        assert_eq!(stated.url.as_deref(), Some("http://www.xfire.com/"));
    }
}
