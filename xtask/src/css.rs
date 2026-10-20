//! A small CSS reader for the style gates (`theme-literals`, `sec-surfaces`,
//! `declared-grounds`; plan E7.7.0).
//!
//! It knows exactly what those gates need and no more: every `{…}` block
//! with its prelude (a selector list or an at-rule), the at-rules and rules
//! that enclose it, and the declarations written directly inside it, each
//! with its line. Comments are blanked first; strings and parentheses are
//! respected, so a `;` inside `url("…")` does not end a declaration.

use std::path::Path;

use anyhow::{Context, Result};

use crate::walk_source_files;

/// One `property: value` written directly inside a block.
#[derive(Debug, Clone)]
pub(crate) struct Decl {
    /// Lower-cased property name (`color`, `--color-xfire-text`).
    pub prop: String,
    pub value: String,
    pub line: usize,
}

/// One `prelude { … }` block.
#[derive(Debug, Clone)]
pub(crate) struct Block {
    /// The selector list or at-rule before the `{`, whitespace collapsed.
    pub prelude: String,
    pub decls: Vec<Decl>,
    /// The preludes of the enclosing blocks, outermost first.
    pub ancestors: Vec<String>,
    /// The line of the prelude.
    pub line: usize,
}

impl Block {
    /// A style rule, not an at-rule block such as `@media` or `@theme`.
    pub(crate) fn is_rule(&self) -> bool {
        !self.prelude.starts_with('@')
    }

    /// Whether this block sits in (or is) `@layer <name>`.
    pub(crate) fn in_layer(&self, name: &str) -> bool {
        self.ancestors
            .iter()
            .chain(std::iter::once(&self.prelude))
            .any(|p| layer_names(p).any(|n| n == name))
    }

    /// Whether this block is a keyframe step (`0%`, `from`, …).
    pub(crate) fn in_keyframes(&self) -> bool {
        self.ancestors.iter().any(|p| p.starts_with("@keyframes"))
    }

    /// The selectors of a rule's prelude, split at top-level commas.
    pub(crate) fn selectors(&self) -> Vec<String> {
        split_top_level(&self.prelude, ',')
            .into_iter()
            .map(|s| collapse_ws(&s))
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// The names a `@layer a, b { … }` prelude opens (empty for anything else).
fn layer_names(prelude: &str) -> impl Iterator<Item = &str> {
    prelude
        .strip_prefix("@layer")
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
}

/// Replace every `/* … */` with spaces, keeping newlines (and so lines).
pub(crate) fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("*/").map_or(after.len(), |e| e + 2);
        let comment = &rest[start..start + 2 + end];
        out.extend(comment.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
        rest = &rest[start + 2 + end..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Split at `sep` outside parentheses, brackets and strings.
pub(crate) fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut cur = String::new();
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                _ if c == sep && depth == 0 => {
                    parts.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            },
        }
        cur.push(c);
    }
    parts.push(cur);
    parts
}

struct Frame {
    prelude: String,
    line: usize,
    decls: Vec<Decl>,
}

/// Every block of a stylesheet, in source order.
pub(crate) fn blocks(src: &str) -> Vec<Block> {
    let text = strip_comments(src);
    let line_at = |byte: usize| text[..byte].matches('\n').count() + 1;
    let mut out: Vec<(usize, Block)> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut seg_start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;

    // A segment's line is the line of its first non-blank character.
    let seg_line = |from: usize, to: usize| {
        let seg = &text[from..to];
        let lead = seg.len() - seg.trim_start().len();
        line_at(from + lead)
    };
    let push_decl = |stack: &mut Vec<Frame>, from: usize, to: usize| {
        let seg = text[from..to].trim();
        let Some(frame) = stack.last_mut() else {
            return;
        };
        if let Some((prop, value)) = seg.split_once(':') {
            frame.decls.push(Decl {
                prop: prop.trim().to_ascii_lowercase(),
                value: collapse_ws(value),
                line: seg_line(from, to),
            });
        }
    };

    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => depth += 1,
            ')' => depth -= 1,
            '{' if depth == 0 => {
                stack.push(Frame {
                    prelude: collapse_ws(&text[seg_start..i]),
                    line: seg_line(seg_start, i),
                    decls: Vec::new(),
                });
                seg_start = i + 1;
            }
            ';' if depth == 0 => {
                push_decl(&mut stack, seg_start, i);
                seg_start = i + 1;
            }
            '}' if depth == 0 => {
                push_decl(&mut stack, seg_start, i);
                seg_start = i + 1;
                if let Some(frame) = stack.pop() {
                    let ancestors = stack.iter().map(|f| f.prelude.clone()).collect();
                    out.push((
                        frame.line,
                        Block {
                            prelude: frame.prelude,
                            decls: frame.decls,
                            ancestors,
                            line: frame.line,
                        },
                    ));
                }
            }
            _ => {}
        }
    }
    out.sort_by_key(|(line, _)| *line);
    out.into_iter().map(|(_, b)| b).collect()
}

/// The custom properties a value reads: `var(--a, var(--b))` → `--a`, `--b`.
pub(crate) fn var_refs(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(at) = rest.find("var(") {
        let after = rest[at + 4..].trim_start();
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if name.starts_with("--") {
            out.push(name);
        }
        rest = &rest[at + 4..];
    }
    out
}

/// Every stylesheet under `src/styles`, as `(relative path, source)`.
pub(crate) fn stylesheets(root: &Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for path in walk_source_files(&root.join("src/styles"), &["css"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        out.push((rel, src));
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_blocks_with_their_layers_and_lines() {
        let css = "@layer components {\n  /* a; b { */\n  .a, .b:hover {\n    color: var(--x, var(--y));\n    background: url(\"a;b\");\n  }\n  @media (prefers-contrast: more) {\n    :root { --z: #fff }\n  }\n}\n@keyframes k { 0% { color: red } }";
        let b = blocks(css);
        let rule = b.iter().find(|b| b.prelude == ".a, .b:hover").unwrap();
        assert_eq!(rule.selectors(), vec![".a", ".b:hover"]);
        assert!(rule.in_layer("components") && !rule.in_layer("sec"));
        assert_eq!(rule.decls.len(), 2);
        assert_eq!(rule.decls[0].line, 4);
        assert_eq!(var_refs(&rule.decls[0].value), vec!["--x", "--y"]);
        assert_eq!(rule.decls[1].value, "url(\"a;b\")");
        let root = b.iter().find(|b| b.prelude == ":root").unwrap();
        assert_eq!(root.decls[0].prop, "--z");
        assert!(root.in_layer("components"));
        let step = b.iter().find(|b| b.prelude == "0%").unwrap();
        assert!(step.in_keyframes());
        assert_eq!(rule.line, 3);
    }
}
