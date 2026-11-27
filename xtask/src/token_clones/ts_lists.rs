//! The TS/TSX list rule (lead ruling 2026-11-26, from rekindle-38's
//! `ForumChannelView.tsx` <> `MemberProfilePopup.tsx` finding): a run of
//! independent single-variable declarations (`const [x, setX] =
//! createSignal(…);` one after another) is a list, the TS form of the Rust
//! import and lint lists. A clone whose two sides are such runs in two
//! different files shares no logic and is not a clone. The same run repeated
//! inside one file stays a finding: there it is a missing object (five drafts
//! seeded alike), so the caller applies this rule to cross-file pairs only.

/// The control-flow words a declaration list never holds.
const CONTROL: [&str; 10] = [
    "if", "else", "for", "while", "do", "switch", "case", "return", "try", "throw",
];

/// True when `text` is only independent single-variable declarations: each
/// statement a `const`/`let`/`var` with one declarator, no arrow function
/// and no control flow, none reading a name an earlier one declared.
pub(crate) fn independent_declarations(text: &str) -> bool {
    let Some(stmts) = statements(text) else {
        return false;
    };
    let mut declared: Vec<String> = Vec::new();
    for stmt in &stmts {
        let Some(rest) = ["const ", "let ", "var "]
            .iter()
            .find_map(|k| stmt.strip_prefix(k))
        else {
            return false;
        };
        let Some((pattern, init)) = split_binding(rest) else {
            return false;
        };
        let words = words(init);
        if init.contains("=>")
            || top_level(init, ',')
            || words
                .iter()
                .any(|w| CONTROL.contains(w) || declared.iter().any(|d| d == w))
        {
            return false;
        }
        let names = pattern.split(':').next().unwrap_or(pattern);
        declared.extend(self::words(names).into_iter().map(str::to_owned));
    }
    !stmts.is_empty()
}

/// The statements of `text` split at `;` and line ends outside brackets and
/// strings, or `None` when a bracket is left open or closed unopened.
fn statements(text: &str) -> Option<Vec<String>> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    let (mut depth, mut quote) = (0usize, None::<char>);
    for c in text.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (None, '"' | '\'' | '`') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth = depth.checked_sub(1)?,
            (None, ';' | '\n') if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    (depth == 0 && quote.is_none()).then(|| {
        out.into_iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

/// The binding pattern and the initializer of one declarator: split at the
/// first `=` outside brackets that is not part of `==`, `=>`, `<=`, `>=`
/// or `!=`.
fn split_binding(rest: &str) -> Option<(&str, &str)> {
    let bytes = rest.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth = depth.saturating_sub(1),
            b'=' if depth == 0 => {
                let prev = i.checked_sub(1).and_then(|p| bytes.get(p)).copied();
                let next = bytes.get(i + 1).copied();
                if !matches!(prev, Some(b'=' | b'!' | b'<' | b'>'))
                    && !matches!(next, Some(b'=' | b'>'))
                {
                    return Some((rest[..i].trim(), rest[i + 1..].trim()));
                }
            }
            _ => {}
        }
    }
    None
}

/// True when `c` occurs in `text` outside brackets.
fn top_level(text: &str, c: char) -> bool {
    let mut depth = 0usize;
    text.chars().any(|ch| {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0 && ch == c
    })
}

/// The identifier words of `text`.
fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .filter(|w| !w.is_empty())
        .collect()
}
