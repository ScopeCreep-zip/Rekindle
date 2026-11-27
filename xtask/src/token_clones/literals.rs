//! Keep string values significant when jscpd proposes a structural match.
//! Its function-similarity pass normalizes literals even in strict mode.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use super::{Duplicate, Side};
use crate::rust_lex;

pub(super) struct Literal {
    line: usize,
    last_line: usize,
    value: Vec<u8>,
}

fn rust_value(text: &str) -> Option<Vec<u8>> {
    match syn::parse_str::<syn::Lit>(text).ok()? {
        syn::Lit::Str(literal) => Some(literal.value().into_bytes()),
        syn::Lit::ByteStr(literal) => Some(literal.value()),
        syn::Lit::CStr(literal) => Some(literal.value().into_bytes()),
        _ => None,
    }
}

fn rust_literals(source: &str) -> Vec<Literal> {
    let mut lines = source
        .match_indices('\n')
        .map(|(position, _)| position)
        .peekable();
    let mut line = 1;
    rust_lex::lex(source)
        .literals
        .into_iter()
        .filter_map(|span| {
            let text = &source[span.clone()];
            let value = rust_value(text)?;
            while lines.peek().is_some_and(|next| *next < span.start) {
                line += 1;
                lines.next();
            }
            Some(Literal {
                line,
                last_line: line + text.matches('\n').count(),
                value,
            })
        })
        .collect()
}

fn quoted_end(src: &str, from: usize, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (offset, next) in src[from..].char_indices() {
        if escaped {
            escaped = false;
        } else if next == '\\' {
            escaped = true;
        } else if next == quote {
            return Some(from + offset);
        }
    }
    None
}

fn ts_literals(mut source: &str) -> Vec<Literal> {
    let mut line = 1;
    let mut found = Vec::new();
    while let Some(first) = source.chars().next() {
        let consumed = if source.starts_with("//") {
            source.find('\n').unwrap_or(source.len())
        } else if let Some(comment) = source.strip_prefix("/*") {
            comment.find("*/").map_or(source.len(), |end| end + 4)
        } else if matches!(first, '"' | '\'' | '`') {
            if let Some(end) = quoted_end(source, 1, first) {
                let content = &source[1..end];
                found.push(Literal {
                    line,
                    last_line: line + content.matches('\n').count(),
                    value: content.as_bytes().to_vec(),
                });
                end + 1
            } else {
                first.len_utf8()
            }
        } else {
            first.len_utf8()
        };
        line += source[..consumed].matches('\n').count();
        source = &source[consumed..];
    }
    found
}

pub(super) fn literals_of(source: &str, format: &str) -> Vec<Literal> {
    if format == "rust" {
        rust_literals(source)
    } else {
        ts_literals(source)
    }
}

pub(super) fn string_values(
    literals: &[Literal],
    start: usize,
    end: usize,
) -> impl Iterator<Item = &[u8]> {
    literals
        .iter()
        .filter(move |literal| literal.line <= end && literal.last_line >= start)
        .map(|literal| literal.value.as_slice())
}

fn cached<'a>(
    cache: &'a mut HashMap<String, Vec<Literal>>,
    root: &Path,
    side: &Side,
    format: &str,
) -> Result<&'a [Literal]> {
    Ok(match cache.entry(side.name.clone()) {
        Entry::Occupied(held) => held.into_mut(),
        Entry::Vacant(slot) => {
            let source = std::fs::read_to_string(root.join(&side.name))?;
            slot.insert(literals_of(&source, format))
        }
    })
}

pub(super) fn split_different_literals(
    root: &Path,
    candidates: Vec<Duplicate>,
) -> Result<(Vec<Duplicate>, usize)> {
    let mut files = HashMap::new();
    let mut kept = Vec::new();
    let mut different = 0;
    for candidate in candidates {
        let first = &candidate.first;
        let second = &candidate.second;
        let left: Vec<_> = string_values(
            cached(&mut files, root, first, &candidate.format)?,
            first.start,
            first.end,
        )
        .map(<[u8]>::to_vec)
        .collect();
        let right = cached(&mut files, root, second, &candidate.format)?;
        if left
            .iter()
            .map(Vec::as_slice)
            .eq(string_values(right, second.start, second.end))
        {
            kept.push(candidate);
        } else {
            different += 1;
        }
    }
    Ok((kept, different))
}

#[cfg(test)]
#[path = "literals_tests.rs"]
mod tests;
