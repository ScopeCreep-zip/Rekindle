//! `check-no-shell-permission-bits`: the shell declares no permission bit
//! (contract PN1). Which permissions exist, their groups and labels come
//! from the Host's catalogue (`get_permission_catalogue`), and what the user
//! may do from CAPR's denied maps, so a BigInt shift literal such as
//! `1n << 20n` in `src/` is a second copy of the Host's policy. This gate
//! walks every `.ts`/`.tsx` file under `src/` and fails on one.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::walk_source_files;

/// Whether `line` holds a BigInt shift literal: digits `n`, `<<`, digits `n`.
pub(crate) fn has_bit_literal(line: &str) -> bool {
    let mut i = 0;
    while let Some(at) = line[i..].find("<<").map(|p| p + i) {
        let before = line[..at].trim_end();
        let after = line[at + 2..].trim_start();
        let left = before.ends_with('n')
            && before[..before.len() - 1].ends_with(|c: char| c.is_ascii_digit());
        let right_digits = after.chars().take_while(char::is_ascii_digit).count();
        let right = right_digits > 0 && after[right_digits..].starts_with('n');
        if left && right {
            return true;
        }
        i = at + 2;
    }
    false
}

/// Every `path:line  text` in `root/src` holding a BigInt shift literal.
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
            if has_bit_literal(line) {
                hits.push(format!("{rel}:{}  {}", n + 1, line.trim()));
            }
        }
    }
    Ok(hits)
}

pub fn check_no_shell_permission_bits(root: &Path) -> Result<()> {
    let hits = scan(root)?;
    if hits.is_empty() {
        println!("  no permission bit is declared in src/ (the catalogue is the Host's)");
        return Ok(());
    }
    for hit in &hits {
        eprintln!("  ✗ {hit}");
    }
    Err(anyhow!(
        "{} permission bit literal(s) in src/: read the Host's catalogue (get_permission_catalogue) and CAPR's denied maps instead (contract PN1)",
        hits.len()
    ))
}

#[cfg(test)]
#[path = "shell_permission_bits_tests.rs"]
mod tests;
