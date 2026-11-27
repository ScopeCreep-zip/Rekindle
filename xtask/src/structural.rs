//! Structural tokens, the clone floor both duplicate passes share (gate
//! hardening item 2, "Item 2 rulings after the triage", 2026-11-26).
//!
//! The triage (`triage-duplicates-2026-11-26.md` §1.f) found 1,668 of 1,689
//! false positives were lists matching lists: under a consistent renaming any
//! list of N distinct paths (imports, lint attributes, enum-variant lists,
//! or-patterns) is a renaming of any other such list, though the two share no
//! logic. A match therefore counts only when its span holds at least
//! `MIN_STRUCTURAL` structural tokens:
//!
//! - a keyword (path roots such as `Self::` are cut with the rest of the
//!   path, so `Self::A, Self::B` holds none);
//! - a literal;
//! - a call: a name directly followed by `(`;
//! - an operator: any punctuation except the list separators `,` `;` `:`
//!   `|` `.` `#` and the arm separator `=>` (a method call counts as a call,
//!   a field access holds no logic of its own).
//!
//! Identifiers and delimiters never count. The near-duplicate pass maps its
//! normalised tokens to `Lexeme`s, the jscpd pass lexes what jscpd saw with
//! `rust_lex`; both count with `count`.

use crate::rust_lex::{self, Tok as LexTok};

/// The fewest structural tokens a clone's span must hold.
pub(crate) const MIN_STRUCTURAL: usize = 10;

/// The Rust keywords (strict and reserved ones that occur as identifiers in
/// a token stream), `true`/`false` and the path roots.
pub(crate) const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield",
];

/// The punctuation that separates list items rather than computing.
const SEPARATORS: [char; 6] = [',', ';', ':', '|', '.', '#'];

/// One token as the floor sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lexeme {
    Keyword,
    Name,
    Literal,
    Punct(char),
    Open(char),
    Close,
}

/// The lexeme of an identifier's text.
pub(crate) fn word(text: &str) -> Lexeme {
    if KEYWORDS.contains(&text) {
        Lexeme::Keyword
    } else if text.starts_with(|c: char| c.is_ascii_digit()) {
        Lexeme::Literal
    } else {
        Lexeme::Name
    }
}

/// The lexeme of one punctuation or delimiter character.
pub(crate) fn punct(c: char) -> Lexeme {
    match c {
        '(' | '[' | '{' => Lexeme::Open(c),
        ')' | ']' | '}' => Lexeme::Close,
        _ => Lexeme::Punct(c),
    }
}

/// The structural tokens in `lexemes`.
pub(crate) fn count(lexemes: &[Lexeme]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while let Some(&l) = lexemes.get(i) {
        let next = lexemes.get(i + 1).copied();
        match l {
            Lexeme::Keyword | Lexeme::Literal => n += 1,
            Lexeme::Name if next == Some(Lexeme::Open('(')) => n += 1,
            Lexeme::Punct('=') if next == Some(Lexeme::Punct('>')) => i += 1,
            Lexeme::Punct(c) if !SEPARATORS.contains(&c) => n += 1,
            _ => {}
        }
        i += 1;
    }
    n
}

/// A Rust text lexed once, so the lexemes of any line span can be read.
pub(crate) struct LexedText {
    /// (1-based line, lexeme) in source order, path prefixes cut.
    lexemes: Vec<(usize, Lexeme)>,
}

impl LexedText {
    pub(crate) fn new(src: &str) -> Self {
        let lexed = rust_lex::lex(src);
        let mut line_starts = vec![0];
        line_starts.extend(src.match_indices('\n').map(|(at, _)| at + 1));
        let line = |at: usize| line_starts.partition_point(|&s| s <= at);
        let mut items: Vec<(usize, usize, Lexeme)> = Vec::new();
        let toks = &lexed.tokens;
        let is_colon = |k: usize| toks.get(k).map(|t| t.kind) == Some(LexTok::Punct(b':'));
        let mut k = 0;
        while let Some(t) = toks.get(k) {
            let lexeme = match t.kind {
                LexTok::Ident if is_colon(k + 1) && is_colon(k + 2) => {
                    // A path segment that is not the last one, and its `::`.
                    k += 3;
                    continue;
                }
                LexTok::Ident => word(&src[t.start..t.end]),
                LexTok::Punct(b) => punct(char::from(b)),
            };
            items.push((t.start, line(t.start), lexeme));
            k += 1;
        }
        items.extend(
            lexed
                .literals
                .iter()
                .map(|r| (r.start, line(r.start), Lexeme::Literal)),
        );
        items.sort_by_key(|&(at, _, _)| at);
        Self {
            lexemes: items.into_iter().map(|(_, l, x)| (l, x)).collect(),
        }
    }

    /// The structural tokens on lines `start..=end`.
    pub(crate) fn structural(&self, start: usize, end: usize) -> usize {
        let span: Vec<Lexeme> = self
            .lexemes
            .iter()
            .filter(|(l, _)| (start..=end).contains(l))
            .map(|&(_, x)| x)
            .collect();
        count(&span)
    }
}

#[cfg(test)]
#[path = "structural_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "test_fixture_sources.rs"]
pub(crate) mod test_fixture_sources;
