fn main() {
    // Make the compile-time target triple available at runtime for sidecar path resolution.
    println!(
        "cargo:rustc-env=REKINDLE_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    tauri_build::build();
}
