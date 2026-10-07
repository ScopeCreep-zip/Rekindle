//! Filenames from peers, made safe to offer in a save dialog.
//!
//! An attachment's filename is chosen by whoever uploaded it. It is shown
//! as the save dialog's default name, so it must not walk out of the chosen
//! directory, hide itself, spoof its extension, or name a Windows device.
//! Follows the OWASP File Upload Cheat Sheet: restrict characters, cap the
//! length, and refuse leading periods, hyphens and spaces.

/// Longest filename most filesystems accept, in bytes.
const MAX_FILENAME_BYTES: usize = 255;
/// Used when nothing of the original name survives.
const FALLBACK_NAME: &str = "attachment";

/// Unicode bidi controls (CWE-451: `evil\u{202E}gpj.exe` renders as
/// `evilexe.jpg`).
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Characters no mainstream filesystem accepts in a name.
fn is_reserved_char(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
}

/// Windows device names, reserved with any extension (`CON.txt`).
fn is_windows_reserved_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| {
            upper
                .strip_prefix(p)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        })
}

/// Cut `s` to at most `max` bytes on a char boundary.
fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A peer-supplied filename reduced to a safe single path component.
///
/// - keeps only the last component after `/` or `\`;
/// - drops control characters, bidi controls and filesystem-reserved
///   characters;
/// - collapses `..` runs to a single `.`;
/// - strips leading `.`, `-` and whitespace, and trailing `.` and whitespace;
/// - prefixes Windows device stems (`CON`, `COM1`, …) with `_`;
/// - caps the name at 255 bytes, keeping the extension;
/// - returns `attachment` when nothing is left.
#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut cleaned: String = last
        .chars()
        .filter(|&c| !c.is_control() && !is_bidi_control(c) && !is_reserved_char(c))
        .collect();
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    let trimmed = cleaned
        .trim_start_matches(|c: char| c == '.' || c == '-' || c.is_whitespace())
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        return FALLBACK_NAME.to_owned();
    }

    let (stem, ext) = match trimmed.rfind('.') {
        Some(i) => (&trimmed[..i], &trimmed[i..]),
        None => (trimmed, ""),
    };
    let stem = if is_windows_reserved_stem(stem) {
        format!("_{stem}")
    } else {
        stem.to_owned()
    };
    let ext = truncate_bytes(ext, MAX_FILENAME_BYTES / 2);
    let stem = truncate_bytes(&stem, MAX_FILENAME_BYTES - ext.len());
    format!("{stem}{ext}")
}

#[cfg(test)]
mod tests {
    use super::sanitize_filename;

    #[test]
    fn strips_paths_and_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a/b"), "b");
        assert_eq!(sanitize_filename("a\\b"), "b");
        assert_eq!(sanitize_filename(".."), "attachment");
        assert_eq!(sanitize_filename("report..pdf"), "report.pdf");
    }

    #[test]
    fn strips_controls_and_spoofing() {
        assert_eq!(sanitize_filename("a\0b.txt"), "ab.txt");
        assert_eq!(sanitize_filename("evil\u{202E}gpj.exe"), "evilgpj.exe");
        assert_eq!(sanitize_filename("what?.txt"), "what.txt");
        assert_eq!(sanitize_filename("  -.bashrc"), "bashrc");
        assert_eq!(sanitize_filename(".bashrc"), "bashrc");
        assert_eq!(sanitize_filename("notes.txt. "), "notes.txt");
    }

    #[test]
    fn guards_windows_device_names() {
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename("lpt1"), "_lpt1");
        assert_eq!(sanitize_filename("COM10.txt"), "COM10.txt");
        assert_eq!(sanitize_filename("console.txt"), "console.txt");
    }

    #[test]
    fn caps_length_keeping_the_extension() {
        let out = sanitize_filename(&format!("{}.pdf", "a".repeat(300)));
        assert_eq!(out.len(), 255);
        assert_eq!(
            std::path::Path::new(&out).extension(),
            Some(std::ffi::OsStr::new("pdf"))
        );
        let multibyte = sanitize_filename(&"é".repeat(300));
        assert!(multibyte.len() <= 255);
        assert!(multibyte.chars().all(|c| c == 'é'));
    }

    #[test]
    fn empty_input_gets_a_name() {
        assert_eq!(sanitize_filename(""), "attachment");
        assert_eq!(sanitize_filename("\u{202E}\u{0}"), "attachment");
    }
}
