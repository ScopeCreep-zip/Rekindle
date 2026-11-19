//! A test-only zip writer (PKWARE APPNOTE 6.3.10 §4.3) that can craft every
//! case the walk refuses, plus the minimal valid skin.

use rekindle_types::dto::skin::{SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX};
use serde_json::{json, Value};

/// Unix "made by" host, version 2.0.
pub const MADE_BY_UNIX: u16 = 0x0314;
pub const FLAG_UTF8: u16 = 1 << 11;

/// One entry as it is written: every header field the tests vary.
#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
    pub method: u16,
    pub flags: u16,
    pub made_by: u16,
    pub external_attrs: u32,
    /// Overrides the computed CRC-32.
    pub crc: Option<u32>,
}

impl Entry {
    /// A regular Unix file, mode 0644.
    pub fn file(name: &str, data: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            data: data.to_vec(),
            method: 0,
            flags: if name.is_ascii() { 0 } else { FLAG_UTF8 },
            made_by: MADE_BY_UNIX,
            external_attrs: 0o100_644 << 16,
            crc: None,
        }
    }

    /// A Unix directory entry, mode 0755 (`name` ends with `/`).
    pub fn dir(name: &str) -> Self {
        Self {
            external_attrs: 0o040_755 << 16,
            ..Self::file(name, b"")
        }
    }

    fn crc(&self) -> u32 {
        self.crc.unwrap_or_else(|| crc32fast::hash(&self.data))
    }

    /// The local header followed by the data.
    pub fn local(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x21u16.to_le_bytes());
        out.extend_from_slice(&self.crc().to_le_bytes());
        let len = u32::try_from(self.data.len()).unwrap();
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&u16::try_from(self.name.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    fn central(&self, offset: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        out.extend_from_slice(&self.made_by.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x21u16.to_le_bytes());
        out.extend_from_slice(&self.crc().to_le_bytes());
        let len = u32::try_from(self.data.len()).unwrap();
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&u16::try_from(self.name.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attrs
        out.extend_from_slice(&self.external_attrs.to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        out
    }
}

/// An archive under construction: local records, then the directory.
#[derive(Default)]
pub struct Zip {
    locals: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl Zip {
    /// Writes the entry's local record and its central record; returns the
    /// local header's offset.
    pub fn add(&mut self, entry: &Entry) -> usize {
        let offset = self.locals.len();
        self.locals.extend_from_slice(&entry.local());
        self.add_central(entry, offset);
        offset
    }

    /// Writes only a central record pointing at `offset`.
    pub fn add_central(&mut self, entry: &Entry, offset: usize) {
        self.central.extend_from_slice(&entry.central(offset));
        self.count += 1;
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out = self.locals;
        let cd_offset = u32::try_from(out.len()).unwrap();
        out.extend_from_slice(&self.central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]); // disk numbers
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.central.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}

/// A zip of exactly these entries.
pub fn zip(entries: &[Entry]) -> Vec<u8> {
    let mut z = Zip::default();
    for e in entries {
        z.add(e);
    }
    z.finish()
}

/// The minimal valid manifest.
pub fn manifest() -> Value {
    json!({
        "format": format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        "id": "classic-xfire",
        "name": "Classic Xfire",
        "description": "The 2008 Symbiosis look.",
        "license": { "spdx": "MIT" },
        "variants": [{ "base": "dark", "contrast": "standard", "tokens": "dark.json" }]
    })
}

/// The minimal valid package around `manifest`, plus `extra` entries.
pub fn package(manifest: &Value, extra: &[Entry]) -> Vec<u8> {
    let mut entries = vec![
        Entry::file("manifest.json", manifest.to_string().as_bytes()),
        Entry::file("dark.json", b"{}"),
        Entry::dir("assets/"),
        Entry::file("assets/frame.png", b"png"),
    ];
    entries.extend_from_slice(extra);
    zip(&entries)
}

/// The plan E7.7.7 surface slots on one test element each (labelled
/// slots with one label `.<slot>-label`), for a complete region map.
pub fn surface_entries() -> Vec<(String, rekindle_skin::regions::css::RegionMap)> {
    use rekindle_skin::regions::css::RegionMap;
    rekindle_skin::regions::SLOTS
        .iter()
        .filter(|s| s.css && (s.id.starts_with("buddylist.") || s.id.starts_with("statusbar")))
        .map(|s| {
            let el = format!(".{}", s.id.replace('.', "-"));
            let map = if s.labelled {
                RegionMap::labelled(&[&el], &[&format!("{el}-label")])
            } else {
                RegionMap::elements(&[&el])
            };
            (s.id.to_owned(), map)
        })
        .collect()
}
