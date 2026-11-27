//! `check-no-key-slices`: the shell never names anyone by a slice of a key
//! (contracts VN1, VN2). A member, a peer, a sender and our own identity are
//! named as the Host serves them (the shown name, else the fingerprint form),
//! so `<key>.slice(0, 8)` in `src/` is a raw key fragment reaching the user.
//! This gate walks every `.ts`/`.tsx` file under `src/` and fails on a
//! `.slice(0, 8)` whose receiver names a key, pseudonym, pubkey, sender or
//! peer. There is no skip list.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::walk_source_files;

/// Words that make the sliced value a key.
const KEY_WORDS: &[&str] = &["key", "pseudonym", "pubkey", "sender", "peer"];

/// Whether `line` slices the first 8 characters of a value that names a key.
pub(crate) fn slices_a_key(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    let mut from = 0;
    while let Some(at) = compact[from..].find(".slice(0,8)").map(|p| p + from) {
        // The receiver: the identifier chain just before the dot.
        let receiver: String = compact[..at]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '?' | '!' | '$'))
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let lower = receiver.to_ascii_lowercase();
        if KEY_WORDS.iter().any(|w| lower.contains(w)) {
            return true;
        }
        from = at + 1;
    }
    false
}

/// Every `path:line  text` in `root/src` that slices a key.
pub(crate) fn scan(root: &Path) -> Result<Vec<String>> {
    let src = root.join("src");
    let mut hits = Vec::new();
    for file in walk_source_files(&src, &["ts", "tsx"])? {
        let text = std::fs::read_to_string(&file)?;
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for (n, line) in text.lines().enumerate() {
            if slices_a_key(line) {
                hits.push(format!("{rel}:{}  {}", n + 1, line.trim()));
            }
        }
    }
    Ok(hits)
}

pub fn check_no_key_slices(root: &Path) -> Result<()> {
    let hits = scan(root)?;
    if hits.is_empty() {
        println!("  no key is sliced into a name in src/ (names are the Host's)");
        return Ok(());
    }
    for hit in &hits {
        eprintln!("  ✗ {hit}");
    }
    Err(anyhow!(
        "{} key slice(s) in src/: render the Host's shown name or fingerprint form instead (contracts VN1, VN2)",
        hits.len()
    ))
}

#[cfg(test)]
#[path = "key_slices_tests.rs"]
mod tests;
