//! Keep string values significant when jscpd proposes a structural match.
//! Its function-similarity pass normalizes literals even in strict mode.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use anyhow::Result;

use super::{Duplicate, Side};
use crate::rust_lex;

pub(super) struct Literal {
    span: Range<usize>,
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
    rust_lex::lex(source)
        .literals
        .into_iter()
        .filter_map(|span| {
            let text = &source[span.clone()];
            let value = rust_value(text)?;
            Some(Literal { span, value })
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
    let mut offset = 0;
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
                    span: offset..offset + end + 1,
                    value: content.as_bytes().to_vec(),
                });
                end + 1
            } else {
                first.len_utf8()
            }
        } else {
            first.len_utf8()
        };
        offset += consumed;
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
    matched: Range<usize>,
) -> impl Iterator<Item = &[u8]> {
    literals
        .iter()
        .filter(move |literal| literal.span.start < matched.end && matched.start < literal.span.end)
        .map(|literal| literal.value.as_slice())
}

fn load(
    cache: &mut HashMap<String, Vec<Literal>>,
    root: &Path,
    side: &Side,
    format: &str,
) -> Result<()> {
    if let Entry::Vacant(slot) = cache.entry(side.name.clone()) {
        let source = std::fs::read_to_string(root.join(&side.name))?;
        slot.insert(literals_of(&source, format));
    }
    Ok(())
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
        load(&mut files, root, first, &candidate.format)?;
        load(&mut files, root, second, &candidate.format)?;
        let matched = |side: &Side| side.start_loc.position..side.end_loc.position;
        if string_values(&files[&first.name], matched(first))
            .eq(string_values(&files[&second.name], matched(second)))
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
