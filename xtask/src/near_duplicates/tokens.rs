//! Token normalisation for the near-duplicate pass (research
//! `research-xtask-dup-modules-2026-11-26.md` §1.4 step 1, the dupes-core
//! scheme of §1.2): a `syn` token stream is flattened, attributes and doc
//! comments dropped, every path cut to its last segment, every literal
//! erased to its kind, and every identifier that is not a keyword kept only
//! as a name that a window renames to a positional placeholder (consistent
//! renaming, jscpd's `--ignore-identifiers=consistent`): `d = a + b` matches
//! `s = p + q` but not `x = x + x`.

use std::collections::HashMap;

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

/// One normalised token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Tok {
    /// A keyword, punctuation, delimiter or literal kind.
    Fixed(u32),
    /// A non-keyword identifier (its interned text).
    Name(u32),
}

/// Interns the text of fixed tokens and names.
#[derive(Default)]
pub(crate) struct Interner {
    map: HashMap<String, u32>,
}

impl Interner {
    fn id(&mut self, text: &str) -> u32 {
        if let Some(&id) = self.map.get(text) {
            return id;
        }
        let id = u32::try_from(self.map.len()).unwrap_or(u32::MAX);
        self.map.insert(text.to_owned(), id);
        id
    }
}

/// The Rust keywords (strict and reserved ones that occur as identifiers in
/// a token stream), `true`/`false` and the path roots.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield",
];

/// Appends the normalised form of `stream` to `out`.
pub(crate) fn normalise(stream: TokenStream, int: &mut Interner, out: &mut Vec<Tok>) {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    let mut i = 0;
    while let Some(tree) = trees.get(i) {
        match tree {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                // `#[…]` / `#![…]`: an attribute or a doc comment.
                let bang =
                    matches!(trees.get(i + 1), Some(TokenTree::Punct(b)) if b.as_char() == '!');
                let at = if bang { i + 2 } else { i + 1 };
                if matches!(trees.get(at), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket)
                {
                    i = at + 1;
                    continue;
                }
                out.push(Tok::Fixed(int.id("#")));
            }
            TokenTree::Punct(p) => out.push(Tok::Fixed(int.id(&p.as_char().to_string()))),
            TokenTree::Ident(ident) => {
                if path_prefix(&trees, i) {
                    i += 3;
                    continue;
                }
                let text = ident.to_string();
                if KEYWORDS.contains(&text.as_str()) {
                    out.push(Tok::Fixed(int.id(&text)));
                } else {
                    out.push(Tok::Name(int.id(&text)));
                }
            }
            TokenTree::Literal(lit) => {
                let kind = match syn::Lit::new(lit.clone()) {
                    syn::Lit::Str(_) | syn::Lit::ByteStr(_) | syn::Lit::CStr(_) => "<str>",
                    syn::Lit::Byte(_) | syn::Lit::Char(_) => "<char>",
                    syn::Lit::Int(_) => "<int>",
                    syn::Lit::Float(_) => "<float>",
                    _ => "<lit>",
                };
                out.push(Tok::Fixed(int.id(kind)));
            }
            TokenTree::Group(g) => {
                let (open, close) = match g.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                if !open.is_empty() {
                    out.push(Tok::Fixed(int.id(open)));
                }
                normalise(g.stream(), int, out);
                if !close.is_empty() {
                    out.push(Tok::Fixed(int.id(close)));
                }
            }
        }
        i += 1;
    }
}

/// True when the identifier at `i` is followed by `::` (a path segment that
/// is not the last one).
fn path_prefix(trees: &[TokenTree], i: usize) -> bool {
    matches!(
        (trees.get(i + 1), trees.get(i + 2)),
        (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b)))
            if a.as_char() == ':' && a.spacing() == Spacing::Joint && b.as_char() == ':'
    )
}

/// FNV-1a over 64-bit words.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn word(&mut self, w: u64) {
        for b in w.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// The hash of `toks` with every name renamed to the index of its first
/// occurrence in `toks` (consistent renaming within the slice).
pub(crate) fn renamed_hash(toks: &[Tok]) -> u64 {
    let mut seen: Vec<u32> = Vec::new();
    let mut h = Fnv::new();
    for t in toks {
        match *t {
            Tok::Fixed(c) => h.word(u64::from(c)),
            Tok::Name(n) => {
                let idx = seen.iter().position(|&s| s == n).unwrap_or_else(|| {
                    seen.push(n);
                    seen.len() - 1
                });
                h.word((1 << 40) + u64::try_from(idx).unwrap_or(u64::MAX >> 1));
            }
        }
    }
    h.0
}

/// The sorted, deduplicated hashes of every `k`-token window of `toks`, each
/// renamed within itself, so an inserted or deleted statement disturbs only
/// the windows that overlap it.
pub(crate) fn shingles(toks: &[Tok], k: usize) -> Vec<u64> {
    let mut out: Vec<u64> = toks.windows(k).map(renamed_hash).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// |a ∩ b| for two sorted, deduplicated hash lists.
pub(crate) fn shared(a: &[u64], b: &[u64]) -> usize {
    let (mut left, mut right, mut common) = (0, 0, 0);
    while let (Some(x), Some(y)) = (a.get(left), b.get(right)) {
        match x.cmp(y) {
            std::cmp::Ordering::Less => left += 1,
            std::cmp::Ordering::Greater => right += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                left += 1;
                right += 1;
            }
        }
    }
    common
}

/// `num / den` as a ratio in [0, 1] (0 for an empty denominator).
pub(crate) fn ratio(num: usize, den: usize) -> f64 {
    let n = u32::try_from(num).unwrap_or(u32::MAX);
    let d = u32::try_from(den).unwrap_or(u32::MAX);
    if d == 0 {
        0.0
    } else {
        f64::from(n) / f64::from(d)
    }
}
