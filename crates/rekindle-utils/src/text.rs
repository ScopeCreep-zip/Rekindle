//! Text handling for untrusted strings shown to a user, and char-safe
//! shortening. Byte-index slicing (`&s[..n]`) panics inside a multi-byte
//! character; everything here cuts on character boundaries.

/// The first `max_chars` characters of `s` (all of `s` if shorter).
#[must_use]
pub fn prefix(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((end, _)) => &s[..end],
        None => s,
    }
}

/// `s` shortened to its first `head` and last `tail` characters around an
/// ellipsis, or `s` itself when it is no longer than that would be.
#[must_use]
pub fn abbreviate(s: &str, head: usize, tail: usize) -> String {
    let count = s.chars().count();
    if count <= head + tail + 1 {
        return s.to_owned();
    }
    let tail_start = s
        .char_indices()
        .nth(count - tail)
        .map_or(s.len(), |(index, _)| index);
    format!("{}…{}", prefix(s, head), &s[tail_start..])
}

/// Strip control characters and ANSI escape sequences from untrusted text.
///
/// Allows \n and \t (needed for message formatting). Strips:
/// - Individual control characters (C0 set except \n and \t)
/// - Full ANSI CSI sequences: ESC + '[' + params + final byte
/// - Full ANSI OSC sequences: ESC + ']' + ... + ST
///
/// This prevents terminal escape injection from peer-controlled display
/// names, message bodies, channel topics, etc. A partial strip (removing
/// only the ESC byte) leaves broken `[31m` fragments that could confuse
/// terminals or users. We strip the entire sequence.
pub fn sanitize_for_display(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Start of an escape sequence — consume the entire sequence
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: ESC [ <params> <final byte>
                    chars.next(); // consume '['
                                  // Consume parameter bytes (0x30-0x3F) and intermediate bytes (0x20-0x2F)
                                  // until we hit a final byte (0x40-0x7E) or run out of input
                    loop {
                        match chars.peek() {
                            Some(&fc) if ('\x40'..='\x7e').contains(&fc) => {
                                chars.next(); // consume final byte
                                break;
                            }
                            Some(&fc) if ('\x20'..='\x3f').contains(&fc) => {
                                chars.next(); // consume parameter/intermediate byte
                            }
                            _ => break, // malformed sequence — stop consuming
                        }
                    }
                }
                Some(']') => {
                    // OSC sequence: ESC ] ... ST (ST = ESC \ or BEL)
                    chars.next(); // consume ']'
                    loop {
                        match chars.next() {
                            Some('\x07') | None => break, // BEL or EOF terminates OSC
                            Some('\x1b') => {
                                // ESC \ terminates OSC
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                            _ => {} // consume OSC content
                        }
                    }
                }
                _ => {
                    // Other escape — consume just the ESC
                }
            }
        } else if c.is_control() && c != '\n' && c != '\t' {
            // Strip other control characters (NUL, BEL, BS, etc.)
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
