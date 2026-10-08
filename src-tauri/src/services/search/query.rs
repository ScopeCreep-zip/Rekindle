//! FTS5 query input handling — sanitize user input into a safe MATCH expression.
//!
//! The architecture spec (§23.2 line 2681) declares `query: String,
//! // FTS5 query syntax`. We expose two paths:
//!
//! * [`sanitize_to_match`] — wraps every whitespace-delimited token as a
//!   quoted FTS5 phrase and AND-joins them. Reserved FTS5 characters
//!   inside the token become inert because FTS5 only treats them
//!   specially outside of double-quoted strings (per the FTS5 grammar
//!   reference at https://sqlite.org/fts5.html#full_text_query_syntax).
//! * [`build_match_expr`] — convenience that returns `None` for blank
//!   input so the caller can short-circuit.
//!
//! Power-user FTS5 expressions (boolean operators, prefix `*`, NEAR) are
//! out of scope for v1 search input — the cost of keeping the chat-side
//! input safe outweighs the rare power-user case. They can be added
//! later via a dedicated raw entry point without altering this surface.

/// Return `true` if the trimmed query is empty.
pub fn is_blank(q: &str) -> bool {
    q.trim().is_empty()
}

/// Wrap each whitespace-delimited token as a quoted FTS5 phrase, doubling
/// any embedded `"` to escape it (per FTS5 grammar). Empty tokens are
/// dropped.
pub fn sanitize_to_match(query: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for token in query.split_whitespace() {
        let escaped = token.replace('"', "\"\"");
        parts.push(format!("\"{escaped}\""));
    }
    parts.join(" ")
}

/// Returns `Some(match_expr)` for non-blank input, `None` for blank.
pub fn build_match_expr(query: &str) -> Option<String> {
    if is_blank(query) {
        return None;
    }
    let m = sanitize_to_match(query);
    if m.is_empty() {
        None
    } else {
        Some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_returns_none() {
        assert!(build_match_expr("   ").is_none());
        assert!(build_match_expr("").is_none());
    }

    #[test]
    fn plain_words_become_quoted_and_anded() {
        assert_eq!(
            build_match_expr("hello world"),
            Some(r#""hello" "world""#.to_string())
        );
    }

    #[test]
    fn quotes_inside_token_are_doubled() {
        assert_eq!(
            build_match_expr(r#"say"hi"#),
            Some(r#""say""hi""#.to_string())
        );
    }

    #[test]
    fn collapses_extra_whitespace() {
        assert_eq!(
            build_match_expr("  one   two   "),
            Some(r#""one" "two""#.to_string())
        );
    }
}
