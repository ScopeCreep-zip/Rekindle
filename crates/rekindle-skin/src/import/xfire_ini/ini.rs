//! A bounded reader for the Xfire INI era's files (plan E7.7.3 item 2).
//!
//! The grammar is Windows `GetPrivateProfileString`'s, read strictly:
//! `[section]` headers, `key=value` lines, `;` comments and blank lines,
//! CRLF or LF. There are no includes (a value naming another file is only a
//! value; the importer decides which files it reads) and no escapes: a
//! backslash is accepted only as a Windows path separator between two path
//! characters, anything else is an unknown escape and refuses the file.
//! A key outside a section, a duplicate section, a control byte, a line or
//! file past the limits refuse it too. A key repeated in one section is
//! read as Windows reads it, the first occurrence wins (DoiDD repeats ten,
//! `DoiDD Skin.ini:80-81` `Window8.Y`); the later ones are kept out of
//! [`Section::entries`] and listed in [`Ini::repeated`] for the `NOTICE`.

use super::super::ImportError;

/// The largest INI file read (DoiDD's largest is 30 KiB).
pub const MAX_INI_BYTES: usize = 256 * 1024;
/// The longest line, in bytes.
pub const MAX_LINE: usize = 1024;
/// The most lines in one file.
pub const MAX_LINES: usize = 8192;

/// One `key=value`, with its 1-based line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    pub line: usize,
}

/// One `[section]` and its entries, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub entries: Vec<KeyValue>,
}

impl Section {
    /// The value of `key` (keys compare ASCII-case-insensitively, as on
    /// Windows).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.key.eq_ignore_ascii_case(key))
            .map(|e| e.value.as_str())
    }
}

/// A parsed INI file: its sections in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ini {
    pub sections: Vec<Section>,
    /// `[section] key (line n)` of every repeated key not read.
    pub repeated: Vec<String>,
}

impl Ini {
    /// The section `name` (ASCII-case-insensitive).
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

fn path_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b' ')
}

/// A backslash only between two path characters.
fn check_escapes(value: &str) -> Result<(), String> {
    let b = value.as_bytes();
    for (i, _) in b.iter().enumerate().filter(|(_, c)| **c == b'\\') {
        let before = i.checked_sub(1).map(|j| b[j]);
        let after = b.get(i + 1).copied();
        if !(before.is_some_and(path_char) && after.is_some_and(path_char)) {
            return Err(format!("unknown escape at byte {i}"));
        }
    }
    Ok(())
}

fn key_char(c: char) -> bool {
    c.is_ascii_graphic() && !matches!(c, '=' | '[' | ']' | ';')
}

fn section_name(c: char) -> bool {
    key_char(c) || c == ' '
}

/// Parses `bytes` as the INI file `file`.
///
/// # Errors
/// [`ImportError::Format`] naming the line and the rule.
pub fn parse(file: &str, bytes: &[u8]) -> Result<Ini, ImportError> {
    let err = |line: usize, detail: &str| ImportError::Format {
        file: file.to_owned(),
        detail: if line == 0 {
            detail.to_owned()
        } else {
            format!("line {line}: {detail}")
        },
    };
    if bytes.len() > MAX_INI_BYTES {
        return Err(err(0, &format!("larger than {MAX_INI_BYTES} bytes")));
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| err(0, "not UTF-8"))?;
    let mut sections: Vec<Section> = Vec::new();
    let mut repeated = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let n = i + 1;
        if n > MAX_LINES {
            return Err(err(n, &format!("more than {MAX_LINES} lines")));
        }
        if raw.len() > MAX_LINE {
            return Err(err(n, &format!("longer than {MAX_LINE} bytes")));
        }
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.chars().any(|c| c.is_control() && c != '\t') {
            return Err(err(n, "a control character"));
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest
                .strip_suffix(']')
                .filter(|s| !s.trim().is_empty() && s.chars().all(section_name))
                .ok_or_else(|| err(n, "a malformed section header"))?;
            if sections.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
                return Err(err(n, &format!("section [{name}] repeated")));
            }
            sections.push(Section {
                name: name.to_owned(),
                entries: Vec::new(),
            });
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .map(|(k, v)| (k.trim(), v.trim()))
            .filter(|(k, _)| !k.is_empty() && k.chars().all(key_char))
            .ok_or_else(|| err(n, "neither a section, a comment nor key=value"))?;
        check_escapes(value).map_err(|d| err(n, &d))?;
        let section = sections
            .last_mut()
            .ok_or_else(|| err(n, "a key outside any section"))?;
        if section.get(key).is_some() {
            repeated.push(format!("[{}] {key} (line {n})", section.name));
            continue;
        }
        section.entries.push(KeyValue {
            key: key.to_owned(),
            value: value.to_owned(),
            line: n,
        });
    }
    Ok(Ini { sections, repeated })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sections_and_paths() {
        let ini = parse(
            "a.ini",
            b"; c\r\n[Top]\r\nGraphics.top=mainskin\\top.gif\r\nX = 1\r\n\r\n[B]\nk=ALT(\"m\")\n",
        )
        .unwrap();
        let top = ini.section("top").unwrap();
        assert_eq!(top.get("graphics.TOP"), Some("mainskin\\top.gif"));
        assert_eq!(top.get("X"), Some("1"));
        assert_eq!(ini.section("B").unwrap().entries[0].line, 7);
        let ini = parse("a.ini", b"[A]\nk=1\nK=2\n").unwrap();
        assert_eq!(ini.section("A").unwrap().get("k"), Some("1"));
        assert_eq!(ini.repeated, ["[A] K (line 3)"]);
    }

    #[test]
    fn malformed_and_oversize_are_refused() {
        for bad in [
            &b"k=v\n"[..],
            b"[A]\nnot a pair\n",
            b"[A\n",
            b"[A]\n[a]\n",
            b"[A]\nk=a\\\n",
            b"[A]\nk=\\\"x\n",
            b"[A]\nk=a\\\\b\n",
            b"[A]\nk=\x01\n",
            b"[A]\nk=\xff\n",
        ] {
            assert!(parse("a.ini", bad).is_err(), "{bad:?}");
        }
        let long = format!("[A]\nk={}\n", "x".repeat(MAX_LINE));
        assert!(parse("a.ini", long.as_bytes()).is_err());
        let many = "[A]\n".to_owned() + &";\n".repeat(MAX_LINES);
        assert!(parse("a.ini", many.as_bytes()).is_err());
        assert!(parse("a.ini", &vec![b';'; MAX_INI_BYTES + 1]).is_err());
    }
}
