//! The units the near-duplicate pass compares, read from one Rust file with
//! `syn`: every function with a body (free, inherent, trait impl and trait
//! default) and every block's statement list inside them. Line numbers come
//! from the small lexer (`rust_lex`), which locates each `fn name … { … }` in
//! the text, because the xtask does not enable `proc-macro2/span-locations`
//! (see `xtask/Cargo.toml`).

use quote::ToTokens;
use syn::visit::{self, Visit};

use super::tokens::{normalise, Interner, Tok};
use crate::rust_lex::{self, Tok as LexTok};

/// One function with a body.
pub(crate) struct FnUnit {
    pub(crate) name: String,
    /// 1-based lines of its `fn` keyword and its closing brace (0 when the
    /// text could not be located, e.g. a function a macro generates).
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// Its normalised body.
    pub(crate) toks: Vec<Tok>,
    /// The structural tokens in its body (`crate::structural`).
    pub(crate) structural: usize,
}

/// One block's statements, each normalised, inside function `func`; a
/// statement that normalises to nothing (a `use` item) is left out.
pub(crate) struct BlockUnit {
    pub(crate) func: usize,
    pub(crate) stmts: Vec<Vec<Tok>>,
    /// The structural tokens of each statement.
    pub(crate) structural: Vec<usize>,
}

/// A parsed file's units.
#[derive(Default)]
pub(crate) struct FileUnits {
    pub(crate) fns: Vec<FnUnit>,
    pub(crate) blocks: Vec<BlockUnit>,
}

struct Collect<'i> {
    int: &'i mut Interner,
    units: FileUnits,
    stack: Vec<usize>,
}

impl Collect<'_> {
    fn enter(&mut self, name: &syn::Ident, block: &syn::Block) {
        let mut toks = Vec::new();
        normalise(block.to_token_stream(), self.int, &mut toks);
        let structural = self.int.structural(&toks);
        self.units.fns.push(FnUnit {
            name: name.to_string(),
            start: 0,
            end: 0,
            toks,
            structural,
        });
        self.stack.push(self.units.fns.len() - 1);
    }
}

impl<'ast> Visit<'ast> for Collect<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.enter(&f.sig.ident, &f.block);
        visit::visit_item_fn(self, f);
        self.stack.pop();
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.enter(&f.sig.ident, &f.block);
        visit::visit_impl_item_fn(self, f);
        self.stack.pop();
    }
    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        if let Some(block) = &f.default {
            self.enter(&f.sig.ident, block);
            visit::visit_trait_item_fn(self, f);
            self.stack.pop();
        }
    }
    fn visit_block(&mut self, b: &'ast syn::Block) {
        if let Some(&func) = self.stack.last() {
            let stmts: Vec<Vec<Tok>> = b
                .stmts
                .iter()
                .map(|s| {
                    let mut toks = Vec::new();
                    normalise(s.to_token_stream(), self.int, &mut toks);
                    toks
                })
                .filter(|toks| !toks.is_empty())
                .collect();
            let structural = stmts.iter().map(|s| self.int.structural(s)).collect();
            self.units.blocks.push(BlockUnit {
                func,
                stmts,
                structural,
            });
        }
        visit::visit_block(self, b);
    }
}

/// The units of `src`, or `None` when it is not Rust `syn` parses.
pub(crate) fn file_units(src: &str, int: &mut Interner) -> Option<FileUnits> {
    let file = syn::parse_file(src).ok()?;
    let mut c = Collect {
        int,
        units: FileUnits::default(),
        stack: Vec::new(),
    };
    c.visit_file(&file);
    let mut units = c.units;
    let located = locate_fns(src);
    let mut cursor = 0;
    for f in &mut units.fns {
        if let Some(off) = located
            .get(cursor..)
            .and_then(|rest| rest.iter().position(|(n, _, _)| *n == f.name))
        {
            let (_, start, end) = &located[cursor + off];
            f.start = *start;
            f.end = *end;
            cursor += off + 1;
        }
    }
    Some(units)
}

/// Every `fn name … { … }` in the text, in source order: its name and the
/// 1-based lines of `fn` and of the body's closing brace.
fn locate_fns(src: &str) -> Vec<(String, usize, usize)> {
    let lexed = rust_lex::lex(src);
    let toks = &lexed.tokens;
    let text = |i: usize| toks.get(i).map_or("", |t| &src[t.start..t.end]);
    let mut out = Vec::new();
    for i in 0..toks.len() {
        if toks[i].kind != LexTok::Ident || text(i) != "fn" {
            continue;
        }
        if toks.get(i + 1).map(|t| t.kind) != Some(LexTok::Ident) {
            continue;
        }
        let Some(open) = body_open(toks, i + 2) else {
            continue;
        };
        let Some(close) = matching_close(toks, open) else {
            continue;
        };
        out.push((
            text(i + 1).to_owned(),
            rust_lex::line_of(src, toks[i].start),
            rust_lex::line_of(src, toks[close].start),
        ));
    }
    out
}

/// Index of the `{` opening the body after a signature starting at `from`,
/// or `None` when a `;` ends it first (a declaration without a body).
fn body_open(toks: &[rust_lex::Token], from: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, t) in toks.iter().enumerate().skip(from) {
        match t.kind {
            LexTok::Punct(b'(' | b'[') => depth += 1,
            LexTok::Punct(b')' | b']') => depth = depth.saturating_sub(1),
            LexTok::Punct(b'{') if depth == 0 => return Some(j),
            LexTok::Punct(b';') if depth == 0 => return None,
            _ => {}
        }
    }
    None
}

/// Index of the `}` matching the `{` at `open`.
fn matching_close(toks: &[rust_lex::Token], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, t) in toks.iter().enumerate().skip(open) {
        match t.kind {
            LexTok::Punct(b'{') => depth += 1,
            LexTok::Punct(b'}') => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}
