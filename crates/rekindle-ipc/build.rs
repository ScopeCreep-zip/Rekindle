//! The bus build id (plan D3.2): a BLAKE3 hash of every source file that
//! shapes a frame — this crate's wire protocol and message envelope, and
//! the `rekindle-types` vocabulary they carry. Two binaries built from the
//! same sources agree on it; any change to those sources changes it, so a
//! client and host from different builds refuse each other at the
//! handshake with a typed error instead of misreading positional frames.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let roots = [
        manifest.join("src/protocol"),
        manifest.join("src/message.rs"),
        manifest.join("../rekindle-types/src"),
    ];
    let mut files = Vec::new();
    for root in &roots {
        println!("cargo:rerun-if-changed={}", root.display());
        if root.is_dir() {
            rust_files(root, &mut files);
        } else {
            files.push(root.clone());
        }
    }
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for file in &files {
        let relative = file.strip_prefix(&manifest).unwrap_or(file);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(&std::fs::read(file).expect("source file readable"));
    }
    let id = hasher.finalize().to_hex();
    println!(
        "cargo:rustc-env=REKINDLE_IPC_BUILD_ID={}",
        &id.as_str()[..16]
    );
}
