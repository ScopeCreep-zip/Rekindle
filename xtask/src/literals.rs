//! Every string and byte-string literal in a parsed file, including the
//! literals inside macro invocations (`format!`, `assert_eq!`, …), which
//! `syn` leaves as raw token streams. Comments are not literals, so docs
//! can still quote what a gate rejects.

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

struct Walker<'f> {
    /// Called with the literal's value and its token as written (quotes,
    /// `b` prefix, escapes).
    found: &'f mut dyn FnMut(&str, &str),
}

impl Walker<'_> {
    fn scan_tokens(&mut self, tokens: TokenStream) {
        for tt in tokens {
            match tt {
                TokenTree::Group(g) => self.scan_tokens(g.stream()),
                TokenTree::Literal(lit) => {
                    let token = lit.to_string();
                    match syn::parse_str::<syn::Lit>(&token) {
                        Ok(syn::Lit::Str(s)) => (self.found)(&s.value(), &token),
                        Ok(syn::Lit::ByteStr(b)) => {
                            (self.found)(&String::from_utf8_lossy(&b.value()), &token);
                        }
                        _ => {}
                    }
                }
                TokenTree::Ident(_) | TokenTree::Punct(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for Walker<'_> {
    fn visit_lit_str(&mut self, s: &'ast syn::LitStr) {
        (self.found)(&s.value(), &s.token().to_string());
    }

    fn visit_lit_byte_str(&mut self, b: &'ast syn::LitByteStr) {
        (self.found)(&String::from_utf8_lossy(&b.value()), &b.token().to_string());
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        self.scan_tokens(m.tokens.clone());
    }
}

/// Call `found(value, token)` for every string literal in `file`.
pub(crate) fn for_each(file: &syn::File, mut found: impl FnMut(&str, &str)) {
    Walker { found: &mut found }.visit_file(file);
}

/// The line of `token`'s first occurrence in `src` (0 when absent; there is
/// no `span-locations` on proc-macro2, see xtask/Cargo.toml).
pub(crate) fn line_of(src: &str, token: &str) -> usize {
    src.find(token)
        .map_or(0, |at| src[..at].matches('\n').count() + 1)
}
