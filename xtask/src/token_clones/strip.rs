//! The Rust input jscpd reads (gate hardening item 2, "Item 2 rulings after
//! the triage"): every `use` item and every attribute (`#[…]`, `#![…]`) is
//! blanked to spaces, newlines kept, so jscpd never sees an import or a lint
//! list and every line it reports is the source's own line. The
//! near-duplicate pass drops the same items from its token stream
//! (`near_duplicates/tokens.rs`).

use crate::rust_lex::{self, Tok, Token};

/// `src` with its `use` items and attributes blanked.
pub(crate) fn strip_rust(src: &str) -> String {
    let lexed = rust_lex::lex(src);
    let toks = &lexed.tokens;
    let text = |k: usize| toks.get(k).map_or("", |t| &src[t.start..t.end]);
    let is = |k: usize, c: u8| toks.get(k).map(|t| t.kind) == Some(Tok::Punct(c));
    let mut bytes = src.as_bytes().to_vec();
    let mut blank = |from: usize, to: usize| {
        for b in &mut bytes[from..to] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    };
    let mut item_start = true;
    let mut k = 0;
    while let Some(t) = toks.get(k) {
        let end = match t.kind {
            Tok::Punct(b'#') => {
                let open = if is(k + 1, b'!') { k + 2 } else { k + 1 };
                if is(open, b'[') {
                    closing(toks, open, b'[', b']')
                } else {
                    None
                }
            }
            Tok::Ident if item_start && text(k) == "use" => use_end(toks, k),
            _ => None,
        };
        if let Some(last) = end {
            blank(t.start, toks[last].end);
            item_start = true;
            k = last + 1;
            continue;
        }
        // An item may start after `;`, a brace, `pub` or `pub(…)`.
        item_start = matches!(t.kind, Tok::Punct(b';' | b'{' | b'}'))
            || text(k) == "pub"
            || (is(k, b')') && k >= 3 && text(k - 3) == "pub" && is(k - 2, b'('));
        k += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| src.to_owned())
}

/// Index of the delimiter closing the one at `open`.
fn closing(toks: &[Token], open: usize, inc: u8, dec: u8) -> Option<usize> {
    let mut depth = 0usize;
    for (j, t) in toks.iter().enumerate().skip(open) {
        if t.kind == Tok::Punct(inc) {
            depth += 1;
        } else if t.kind == Tok::Punct(dec) {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

/// Index of the `;` ending the `use` item at `at` (braces of a use tree
/// nest; a `;` inside one cannot occur).
fn use_end(toks: &[Token], at: usize) -> Option<usize> {
    toks.iter()
        .enumerate()
        .skip(at)
        .find(|(_, t)| t.kind == Tok::Punct(b';'))
        .map(|(j, _)| j)
}
