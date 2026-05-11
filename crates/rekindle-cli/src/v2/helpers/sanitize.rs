//! Terminal escape injection prevention.
//!
//! Strips ALL ANSI escape sequences and C0 control characters from
//! peer-controlled text (display names, message bodies, channel topics).
//! Preserves \n and \t for message formatting.
//!
//! Handled sequence types:
//! - CSI (ESC [) — colors, cursor movement, erase
//! - OSC (ESC ]) — title set, hyperlinks
//! - DCS (ESC P) — device control strings, sixel, tmux passthrough
//! - APC (ESC _) — application program command
//! - PM  (ESC ^) — privacy message
//! - SS2/SS3 (ESC N / ESC O) — single shift
//! - Unknown ESC sequences — consumed (ESC + next byte dropped)

/// Strip control characters and ANSI escape sequences from untrusted text.
pub fn sanitize_for_display(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: ESC [ <params> <final byte>
                    chars.next();
                    loop {
                        match chars.peek() {
                            Some(&fc) if ('\x40'..='\x7e').contains(&fc) => {
                                chars.next();
                                break;
                            }
                            Some(&fc) if ('\x20'..='\x3f').contains(&fc) => {
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                }
                // OSC, DCS, APC, PM all terminate at ST (ESC \ or BEL).
                // Consume everything between the introducer and terminator.
                Some(']' | 'P' | '_' | '^') => {
                    chars.next(); // consume the introducer character
                    loop {
                        match chars.next() {
                            Some('\x07') | None => break, // BEL or EOF terminates
                            Some('\x1b') => {
                                // ST = ESC backslash
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                            _ => {} // consume payload
                        }
                    }
                }
                // SS2 (ESC N) and SS3 (ESC O) — single shift, consume one char after
                Some('N' | 'O') => {
                    chars.next(); // consume N/O
                    chars.next(); // consume the shifted character
                }
                // Any other ESC sequence — consume ESC + next byte (safe default)
                Some(_) => {
                    chars.next();
                }
                // Bare ESC at end of string — consumed
                None => {}
            }
        } else if c.is_control() && c != '\n' && c != '\t' {
            // Strip C0 control characters (NUL, BEL, BS, etc.)
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_color() {
        assert_eq!(sanitize_for_display("hello\x1b[31mworld"), "helloworld");
    }

    #[test]
    fn strips_sgr_reset() {
        assert_eq!(sanitize_for_display("a\x1b[0mb"), "ab");
    }

    #[test]
    fn strips_cursor_movement() {
        assert_eq!(sanitize_for_display("before\x1b[10Aafter"), "beforeafter");
    }

    #[test]
    fn strips_erase_display() {
        assert_eq!(sanitize_for_display("safe\x1b[2Jtext"), "safetext");
    }

    #[test]
    fn strips_osc_title_bel() {
        assert_eq!(sanitize_for_display("before\x1b]0;evil title\x07after"), "beforeafter");
    }

    #[test]
    fn strips_osc_title_st() {
        assert_eq!(sanitize_for_display("a\x1b]0;payload\x1b\\b"), "ab");
    }

    #[test]
    fn strips_dcs_sequence() {
        // DCS (device control string) — used for sixel injection, tmux passthrough
        assert_eq!(sanitize_for_display("before\x1bPtmux;evil\x1b\\after"), "beforeafter");
    }

    #[test]
    fn strips_dcs_bel_terminated() {
        assert_eq!(sanitize_for_display("a\x1bPpayload\x07b"), "ab");
    }

    #[test]
    fn strips_apc_sequence() {
        assert_eq!(sanitize_for_display("a\x1b_application\x1b\\b"), "ab");
    }

    #[test]
    fn strips_pm_sequence() {
        assert_eq!(sanitize_for_display("a\x1b^privacy\x1b\\b"), "ab");
    }

    #[test]
    fn strips_ss2_ss3() {
        // SS2 = ESC N + one char, SS3 = ESC O + one char
        assert_eq!(sanitize_for_display("a\x1bNXb"), "ab");
        assert_eq!(sanitize_for_display("a\x1bOYb"), "ab");
    }

    #[test]
    fn strips_unknown_esc_sequence() {
        // Unknown ESC sequence — ESC + next byte both consumed
        assert_eq!(sanitize_for_display("a\x1bZb"), "ab");
    }

    #[test]
    fn strips_multiple_sequences() {
        assert_eq!(sanitize_for_display("\x1b[1m\x1b[31mbold red\x1b[0m normal"), "bold red normal");
    }

    #[test]
    fn strips_null_bytes() {
        assert_eq!(sanitize_for_display("hello\x00world"), "helloworld");
    }

    #[test]
    fn strips_bell() {
        assert_eq!(sanitize_for_display("ding\x07dong"), "dingdong");
    }

    #[test]
    fn strips_backspace() {
        assert_eq!(sanitize_for_display("abc\x08def"), "abcdef");
    }

    #[test]
    fn preserves_newline() {
        assert_eq!(sanitize_for_display("line1\nline2"), "line1\nline2");
    }

    #[test]
    fn preserves_tab() {
        assert_eq!(sanitize_for_display("col1\tcol2"), "col1\tcol2");
    }

    #[test]
    fn preserves_unicode() {
        assert_eq!(sanitize_for_display("hello 🌍 世界"), "hello 🌍 世界");
    }

    #[test]
    fn preserves_zwj_emoji() {
        let input = "👨\u{200D}👩\u{200D}👧";
        assert_eq!(sanitize_for_display(input), input);
    }

    #[test]
    fn empty_string() {
        assert_eq!(sanitize_for_display(""), "");
    }

    #[test]
    fn only_escape_sequence() {
        assert_eq!(sanitize_for_display("\x1b[31m"), "");
    }

    #[test]
    fn bare_esc_at_end() {
        assert_eq!(sanitize_for_display("text\x1b"), "text");
    }

    #[test]
    fn mixed_attack_sequence() {
        // CSI + DCS + OSC + plain text
        assert_eq!(
            sanitize_for_display("ok\x1b[31m\x1bPevil\x1b\\\x1b]0;title\x07end"),
            "okend"
        );
    }
}
