//! A small Rust lexer for the module gate (`check-inline-modules`) and its
//! fixer (`outline-modules`). It yields identifiers and punctuation with byte
//! offsets and skips comments, strings, byte strings, raw strings and char
//! literals, so a `mod name {` inside a string or a comment is never taken for
//! code. It only locates text: `syn` decides what the code is and validates
//! every rewrite, because the xtask does not enable
//! `proc-macro2/span-locations` (see `xtask/Cargo.toml`).

use std::ops::Range;

/// One significant token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok {
    Ident,
    Punct(u8),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Token {
    pub(crate) kind: Tok,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// A file's tokens and the byte ranges of its string and char literals.
pub(crate) struct Lexed {
    pub(crate) tokens: Vec<Token>,
    pub(crate) literals: Vec<Range<usize>>,
}

/// An inline `mod name { … }` block found in the text.
#[derive(Debug, Clone)]
pub(crate) struct ModBlock {
    pub(crate) name: String,
    /// Offset of the `mod` keyword.
    pub(crate) mod_kw: usize,
    /// Offset just past the module's name.
    pub(crate) name_end: usize,
    /// Offset of the body's `{`.
    pub(crate) open: usize,
    /// Offset of the body's matching `}`.
    pub(crate) close: usize,
    /// Brace depth of the declaration (0 = a file-level item).
    pub(crate) depth: usize,
}

fn is_ident_byte(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric() || c >= 0x80
}

/// Lex `src`.
pub(crate) fn lex(src: &str) -> Lexed {
    let b = src.as_bytes();
    let mut tokens = Vec::new();
    let mut literals = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        if c == b'/' && next == Some(b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && next == Some(b'*') {
            i = block_comment_end(b, i);
        } else if c == b'"' {
            let e = string_end(b, i + 1);
            literals.push(i..e);
            i = e;
        } else if c == b'\'' {
            match char_end(b, i) {
                Some(e) => {
                    literals.push(i..e);
                    i = e;
                }
                // A lifetime or label: its name lexes as an identifier.
                None => i += 1,
            }
        } else if c.is_ascii_digit() {
            i += 1;
            while i < b.len()
                && (is_ident_byte(b[i])
                    || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)))
            {
                i += 1;
            }
        } else if is_ident_byte(c) {
            if let Some(e) = prefixed_literal_end(b, i) {
                literals.push(i..e);
                i = e;
                continue;
            }
            let s = i;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            tokens.push(Token {
                kind: Tok::Ident,
                start: s,
                end: i,
            });
        } else {
            if !c.is_ascii_whitespace() {
                tokens.push(Token {
                    kind: Tok::Punct(c),
                    start: i,
                    end: i + 1,
                });
            }
            i += 1;
        }
    }
    Lexed { tokens, literals }
}

fn block_comment_end(b: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    i
}

/// End (exclusive) of a quoted string whose body starts at `i`.
fn string_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// End (exclusive) of the char literal opening at `i`, or `None` for a
/// lifetime or label.
fn char_end(b: &[u8], i: usize) -> Option<usize> {
    let first = *b.get(i + 1)?;
    if first == b'\\' {
        let mut j = i + 3;
        while j < b.len() && j < i + 16 {
            if b[j] == b'\'' {
                return Some(j + 1);
            }
            j += 1;
        }
        return None;
    }
    let len = match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    (b.get(i + 1 + len) == Some(&b'\'')).then_some(i + 2 + len)
}

/// End (exclusive) of a `b"…"`, `b'…'`, `c"…"`, `r#"…"#`, `br"…"` or
/// `cr"…"` literal starting at `i`, or `None` when `i` starts an identifier
/// (including a raw identifier `r#name`).
fn prefixed_literal_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    if matches!(b[j], b'b' | b'c') {
        j += 1;
        match b.get(j) {
            Some(b'"') => return Some(string_end(b, j + 1)),
            Some(b'\'') if b[i] == b'b' => return char_end(b, j),
            _ => {}
        }
    }
    if b.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let hashes = b[j..].iter().take_while(|&&c| c == b'#').count();
    j += hashes;
    if b.get(j) != Some(&b'"') {
        return None;
    }
    j += 1;
    while j < b.len() {
        if b[j] == b'"'
            && b[j + 1..]
                .iter()
                .take(hashes)
                .filter(|&&c| c == b'#')
                .count()
                == hashes
        {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(b.len())
}

/// Every `mod name {` block in `src`, in source order (pre-order for nested
/// blocks). Blocks inside macro bodies are included; callers align them with
/// what `syn` parsed.
pub(crate) fn mod_blocks(src: &str, lexed: &Lexed) -> Vec<ModBlock> {
    let t = &lexed.tokens;
    let text = |k: usize| &src[t[k].start..t[k].end];
    let mut out = Vec::new();
    let mut depth = 0usize;
    for k in 0..t.len() {
        match t[k].kind {
            Tok::Punct(b'{') => depth += 1,
            Tok::Punct(b'}') => depth = depth.saturating_sub(1),
            Tok::Ident if text(k) == "mod" && k + 2 < t.len() => {
                if t[k + 1].kind != Tok::Ident || t[k + 2].kind != Tok::Punct(b'{') {
                    continue;
                }
                let mut d = 0usize;
                let close = t[k + 2..].iter().find_map(|tok| {
                    match tok.kind {
                        Tok::Punct(b'{') => d += 1,
                        Tok::Punct(b'}') => {
                            d -= 1;
                            if d == 0 {
                                return Some(tok.start);
                            }
                        }
                        _ => {}
                    }
                    None
                });
                if let Some(close) = close {
                    out.push(ModBlock {
                        name: text(k + 1).trim_start_matches("r#").to_owned(),
                        mod_kw: t[k].start,
                        name_end: t[k + 1].end,
                        open: t[k + 2].start,
                        close,
                        depth,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// The 1-based line of byte offset `at`.
pub(crate) fn line_of(src: &str, at: usize) -> usize {
    src[..at.min(src.len())]
        .bytes()
        .filter(|&c| c == b'\n')
        .count()
        + 1
}
