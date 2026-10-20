//! A small JSX reader for the component gates (`destructive-controls`,
//! `sec-surfaces`; plan E7.7.0): every opening tag of a TSX source with its
//! attributes, its line and the text up to its closing tag. Comparisons and
//! generics (`a < b`, `createSignal<Stage>(`) are not tags.

use std::collections::BTreeMap;

/// One JSX opening tag.
#[derive(Debug)]
pub(crate) struct Element {
    pub tag: String,
    pub line: usize,
    /// Attribute name → raw value (`"…"` or `{…}`, or empty when bare).
    pub attrs: BTreeMap<String, String>,
    /// The text between the tag and its closing tag, `None` if self-closing.
    pub children: Option<String>,
    /// The bytes of the whole element, opening tag to closing tag.
    pub span: std::ops::Range<usize>,
}

/// Whether a `<` at `at` opens a JSX tag rather than a comparison or a
/// generic (`createSignal<Stage>(`): a letter follows it (`a < b` has a
/// space) and the character right before it does not end an operand (a
/// generic is written against its type; JSX text such as `Key <code>` has
/// a space).
fn opens_tag(src: &str, at: usize) -> bool {
    let next = src[at + 1..].chars().next();
    if !next.is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let prev = src[..at].chars().next_back();
    !prev.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ')' | ']' | '.'))
}

/// The end of an opening tag starting at `at` (index of its `>`).
fn tag_end(src: &str, at: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, c) in src[at..].char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' | '`' => quote = Some(c),
                '{' => depth += 1,
                '}' => depth -= 1,
                '>' if depth == 0 => return Some(at + i),
                _ => {}
            },
        }
    }
    None
}

fn parse_attrs(inner: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_ascii_alphabetic() || chars[i] == '-') {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len()
            && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '-' | ':' | '_'))
        {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        let mut value = String::new();
        if chars.get(i) == Some(&'=') {
            i += 1;
            let open = chars.get(i).copied();
            let (close, nests) = match open {
                Some('"') => ('"', false),
                Some('\'') => ('\'', false),
                Some('{') => ('}', true),
                _ => (' ', false),
            };
            let vstart = i;
            let mut depth = 0i32;
            while i < chars.len() {
                let c = chars[i];
                if nests && c == '{' {
                    depth += 1;
                } else if c == close && (!nests || depth == 1) && i > vstart {
                    i += 1;
                    break;
                } else if nests && c == '}' {
                    depth -= 1;
                }
                i += 1;
            }
            value = chars[vstart..i.min(chars.len())].iter().collect();
        }
        attrs.insert(name, value);
    }
    attrs
}

/// Every JSX element of a TSX source.
pub(crate) fn elements(src: &str) -> Vec<Element> {
    let mut out = Vec::new();
    for (at, _) in src.match_indices('<') {
        if !opens_tag(src, at) {
            continue;
        }
        let Some(end) = tag_end(src, at) else {
            continue;
        };
        let inner = &src[at + 1..end];
        let tag: String = inner
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
            .collect();
        let self_closing = inner.trim_end().ends_with('/');
        let children = (!self_closing).then(|| closing_content(src, end + 1, &tag));
        let close = children
            .as_ref()
            .map_or(0, |c| c.len() + tag.len() + "</>".len());
        out.push(Element {
            line: src[..at].matches('\n').count() + 1,
            attrs: parse_attrs(&inner[tag.len()..]),
            span: at..(end + 1 + close).min(src.len()),
            tag,
            children,
        });
    }
    out
}

/// The text from `from` to the matching `</tag>`.
fn closing_content(src: &str, from: usize, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 1;
    let mut i = from;
    while let Some(next_close) = src[i..].find(&close) {
        let opens = src[i..i + next_close]
            .match_indices(&open)
            .filter(|(o, _)| {
                src[i + o + open.len()..].starts_with(|c: char| c.is_whitespace() || c == '>')
            })
            .count();
        depth += opens;
        depth -= 1;
        if depth == 0 {
            return src[from..i + next_close].to_string();
        }
        i += next_close + close.len();
    }
    src[from..].to_string()
}

impl Element {
    /// The content a sighted user reads: the children with every element
    /// marked `aria-hidden="true"` (an icon glyph) and every tag removed,
    /// trimmed. Expressions (`{label()}`) count as text.
    pub(crate) fn visible_text(&self) -> String {
        let Some(children) = self.children.as_deref() else {
            return String::new();
        };
        let mut hidden: Vec<std::ops::Range<usize>> = elements(children)
            .into_iter()
            .filter(|e| e.attrs.get("aria-hidden").map(String::as_str) == Some("\"true\""))
            .map(|e| e.span)
            .collect();
        hidden.sort_by_key(|r| r.start);
        let mut text = String::new();
        let mut at = 0;
        for r in hidden {
            if r.start >= at {
                text.push_str(&children[at..r.start]);
                at = r.end;
            }
        }
        text.push_str(&children[at..]);
        let mut out = String::new();
        let mut in_tag = false;
        for (i, c) in text.char_indices() {
            match c {
                '<' if text[i + 1..].starts_with(|n: char| n.is_ascii_alphabetic() || n == '/') => {
                    in_tag = true;
                }
                '>' if in_tag => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        out.trim().to_string()
    }

    /// The class names the element can carry: the words of a static
    /// `class="…"`, or of every string literal in a `class={…}` expression
    /// (`{warning() ? "a" : "b"}` → `a`, `b`).
    pub(crate) fn classes(&self) -> Vec<String> {
        let Some(value) = self.attrs.get("class") else {
            return Vec::new();
        };
        let mut words = Vec::new();
        let mut quote: Option<char> = None;
        let mut literal = String::new();
        for c in value.chars() {
            match quote {
                Some(q) if c == q => {
                    words.extend(literal.split_whitespace().map(str::to_string));
                    literal.clear();
                    quote = None;
                }
                Some(_) => literal.push(c),
                None if matches!(c, '"' | '\'' | '`') => quote = Some(c),
                None => {}
            }
        }
        words.retain(|w| {
            w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        });
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generics_and_comparisons_are_not_tags() {
        let src = "const a = b < c; const [x] = createSignal<Stage>(\"intro\"); type T = Record<string, number>;";
        assert!(elements(src).is_empty());
        let text = "<p>Key fingerprint <code>ab</code></p>";
        assert_eq!(elements(text).len(), 2);
        let nested = "<div class=\"a\"><div>inner</div>tail</div>";
        let els = elements(nested);
        assert_eq!(els[0].children.as_deref(), Some("<div>inner</div>tail"));
    }

    #[test]
    fn classes_come_from_static_and_expression_values() {
        let src = r#"<p class="sec-review-fact x"><b class={on() ? "sec-a" : `sec-b`}>k</b><i class={cls()} /></p>"#;
        let els = elements(src);
        assert_eq!(els[0].classes(), vec!["sec-review-fact", "x"]);
        assert_eq!(els[1].classes(), vec!["sec-a", "sec-b"]);
        assert!(els[2].classes().is_empty());
    }

    #[test]
    fn visible_text_drops_hidden_icons_and_tags() {
        let src =
            r#"<button><span class="nf-icon" aria-hidden="true">{ICON_TRASH}</span></button>"#;
        assert_eq!(elements(src)[0].visible_text(), "");
        let src = r#"<button><span aria-hidden="true">{ICON_TRASH}</span> <b>Delete</b> {name()}</button>"#;
        assert_eq!(elements(src)[0].visible_text(), "Delete {name()}");
        assert_eq!(elements("<i />")[0].visible_text(), "");
        let src = "<p>a<span>b</span>c</p>";
        assert_eq!(elements(src)[0].span, 0..src.len());
    }
}
