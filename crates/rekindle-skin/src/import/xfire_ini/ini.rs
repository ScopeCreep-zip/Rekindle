//! A bounded reader for the Xfire INI era's files (plan E7.7.3 item 2).
//!
//! The grammar is Windows `GetPrivateProfileString`'s, read strictly:
//! `[section]` headers, `key=value` lines, `;` comments and blank lines,
//! CRLF or LF. There are no includes (a value naming another file is only a
//! value; the importer decides which files it reads) and no escapes: a
//! backslash is accepted only as a Windows path separator between two path
//! characters, anything else is an unknown escape and refuses the file.
//! A key outside a section, a malformed header, a control byte, a line or
//! file past the limits refuse it too.
//!
//! Three things are read as Windows reads them (Wine's
//! `dlls/kernel32/profile.c`, the reference reimplementation, read
//! 2026-11-18; Microsoft Learn `GetPrivateProfileString`: "not
//! case-sensitive", `lpDefault` when "the key does not exist"), each line
//! not read listed in [`Ini::repeated`] for the `NOTICE`:
//!
//! * a key repeated in one section: the first occurrence wins (DoiDD
//!   repeats ten, `DoiDD Skin.ini:80-81` `Window8.Y`); `PROFILE_Find`
//!   returns the first matching key;
//! * a repeated section: the first wins and the later one's keys are never
//!   seen. `PROFILE_Load` appends every header as its own section, no
//!   merge, and `PROFILE_Find` "stops at the first matching section name"
//!   (`HL2.ini:478,614` `[ResizeRight]`, `Battlefield2.ini:343,377,408`
//!   `[BvBk]`);
//! * a line that is not a header or comment and has no `=`: `PROFILE_Load`
//!   stores it as a key with a `NULL` value (`else key->value = NULL;`), and
//!   `GetPrivateProfileStringW` returns the default for it
//!   (`(key && key->value) ? key->value : def_val`), so it gives no value:
//!   skipped (`FluidX.ini:337` `Window1.ResizeY.0`, Xglow's
//!   `version.ini:2` `Version 1.0`, Dystopia's credit line).
//!
//! A key may hold inner spaces, as on Windows, where the key is the text
//! before the first `=`, trimmed (GunZ's `Design by -=X-Redeem-DC=-`).

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
    /// Every line Windows reads no value from: `[section] key (line n)` of
    /// a repeated key, `[section] (line n): a repeated section, its keys
    /// not read`, `[section] (line n): no '=', no value`.
    pub repeated: Vec<String>,
    /// The text of every `;` comment line, after the `;`, trimmed, in file
    /// order (a skin's `version.ini` names its authors in them).
    pub comments: Vec<String>,
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

/// Where the keys of the current line go.
enum Dest {
    /// Before the first header.
    Nowhere,
    /// The last section read.
    Last,
    /// A repeated section's: validated, never read.
    Repeated(String),
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
    let mut comments = Vec::new();
    let mut dest = Dest::Nowhere;
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
        if let Some(comment) = line.strip_prefix(';') {
            comments.push(comment.trim().to_owned());
            continue;
        }
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest
                .strip_suffix(']')
                .filter(|s| !s.trim().is_empty() && s.chars().all(section_name))
                .ok_or_else(|| err(n, "a malformed section header"))?;
            if sections.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
                repeated.push(format!(
                    "[{name}] (line {n}): a repeated section, its keys not read"
                ));
                dest = Dest::Repeated(name.to_owned());
                continue;
            }
            sections.push(Section {
                name: name.to_owned(),
                entries: Vec::new(),
            });
            dest = Dest::Last;
            continue;
        }
        let pair = line.split_once('=').map(|(k, v)| (k.trim(), v.trim()));
        let in_section = match &dest {
            Dest::Nowhere => return Err(err(n, "a key outside any section")),
            Dest::Last => sections.last().map(|s| s.name.clone()),
            Dest::Repeated(name) => Some(name.clone()),
        };
        let Some((key, value)) = pair else {
            repeated.push(format!(
                "[{}] (line {n}): no '=', no value",
                in_section.unwrap_or_default()
            ));
            continue;
        };
        if key.is_empty() || !key.chars().all(|c| key_char(c) || c == ' ') {
            return Err(err(n, "neither a section, a comment nor key=value"));
        }
        check_escapes(value).map_err(|d| err(n, &d))?;
        let (Dest::Last, Some(section)) = (&dest, sections.last_mut()) else {
            continue;
        };
        if section.get(key).is_some() && !position_key(key) {
            repeated.push(format!("[{}] {key} (line {n})", section.name));
            continue;
        }
        section.entries.push(KeyValue {
            key: key.to_owned(),
            value: value.to_owned(),
            line: n,
        });
    }
    Ok(Ini {
        sections,
        repeated,
        comments,
    })
}

/// A layout position key, `WindowN.X`/`WindowN.Y`: Xfire reads one set
/// twice as a sibling's edge then an offset from it (XGlow `Xglow.ini:53-56`
/// `Window7.X=Top.RIGHT`, `Window7.X=-80`), so both entries are kept and
/// `title::windows` joins them; any other repeated key is dropped.
fn position_key(key: &str) -> bool {
    key.get(..6)
        .is_some_and(|p| p.eq_ignore_ascii_case("Window"))
        && key[6..].split_once('.').is_some_and(|(n, attr)| {
            n.parse::<u32>().is_ok()
                && (attr.eq_ignore_ascii_case("X") || attr.eq_ignore_ascii_case("Y"))
        })
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

    /// Wine `PROFILE_Find`: the first section of a name is the one read.
    #[test]
    fn a_repeated_section_reads_the_first() {
        let ini = parse("a.ini", b"[R]\nMinWidth=10\n[x]\n[r]\nMinWidth=60\nNew=1\n").unwrap();
        assert_eq!(ini.sections.len(), 2);
        let r = ini.section("R").unwrap();
        assert_eq!(r.get("MinWidth"), Some("10"));
        assert_eq!(r.get("New"), None);
        assert_eq!(
            ini.repeated,
            ["[r] (line 4): a repeated section, its keys not read"]
        );
    }

    /// Wine `PROFILE_Load` stores a line without `=` with a `NULL` value,
    /// and `GetPrivateProfileStringW` returns the default for it.
    #[test]
    fn a_line_without_equals_gives_no_value() {
        let ini = parse("v.ini", b"[Version]\r\nVersion 1.0\r\nLongName=XGlow\r\n").unwrap();
        let v = ini.section("Version").unwrap();
        assert_eq!(v.get("Version"), None);
        assert_eq!(v.get("Version 1.0"), None);
        assert_eq!(v.get("LongName"), Some("XGlow"));
        assert_eq!(ini.repeated, ["[Version] (line 2): no '=', no value"]);
    }

    /// The key is the text before the first `=`, trimmed, inner spaces kept.
    #[test]
    fn a_key_may_hold_spaces() {
        let ini = parse("v.ini", b"[Version]\nDesign by -=X-Redeem-DC=-\n").unwrap();
        let v = ini.section("Version").unwrap();
        assert_eq!(v.get("design by -"), Some("X-Redeem-DC=-"));
    }

    #[test]
    fn malformed_and_oversize_are_refused() {
        for bad in [
            &b"k=v\n"[..],
            b"[A]\n=v\n",
            b"[A]\nk]=v\n",
            b"not a pair\n",
            b"[A\n",
            b"[A]\n[a]\nk=\\\n",
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
