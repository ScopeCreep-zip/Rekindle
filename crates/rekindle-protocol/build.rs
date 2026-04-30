use std::path::Path;

/// Find the capnp binary, checking standard installation locations on Windows
fn find_capnp() -> Option<std::path::PathBuf> {
    // First, try the PATH
    if let Ok(output) = std::process::Command::new("capnp")
        .arg("--version")
        .output()
    {
        if output.status.success() {
            return Some(std::path::PathBuf::from("capnp"));
        }
    }

    // Standard Homebrew locations on macOS.
    #[cfg(target_os = "macos")]
    {
        for candidate in ["/opt/homebrew/bin/capnp", "/usr/local/bin/capnp"] {
            let capnp = Path::new(candidate);
            if capnp.exists() {
                if let Ok(output) = std::process::Command::new(capnp).arg("--version").output() {
                    if output.status.success() {
                        return Some(capnp.to_path_buf());
                    }
                }
            }
        }
    }

    // On Windows, check the standard installation location from setup-windows.ps1
    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let capnp_dir = Path::new(&local_app_data).join("capnproto");
            if capnp_dir.exists() {
                // Find the tools directory (e.g., capnproto-tools-win32-1.0.2)
                if let Ok(entries) = std::fs::read_dir(&capnp_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            if name.starts_with("capnproto-tools-") {
                                let capnp_exe = path.join("capnp.exe");
                                if capnp_exe.exists() {
                                    // Verify it works
                                    if let Ok(output) = std::process::Command::new(&capnp_exe)
                                        .arg("--version")
                                        .output()
                                    {
                                        if output.status.success() {
                                            return Some(capnp_exe);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(capnp_codegen)");

    // Cap'n Proto schema compilation — requires `capnp` CLI tool.
    // Install via:
    //   Windows: scripts/setup-windows.ps1
    //   macOS:   brew install capnp
    //   Linux:   apt install capnproto (or nix-shell -p capnproto)
    if let Some(capnp_path) = find_capnp() {
        if let Ok(output) = std::process::Command::new(&capnp_path)
            .arg("--version")
            .output()
        {
            let version = String::from_utf8_lossy(&output.stdout);
            println!("cargo:warning=Using capnp: {}", version.trim());

            // If capnp is not in PATH, add its directory to PATH for capnpc
            if capnp_path.to_string_lossy() != "capnp" {
                if let Some(parent) = capnp_path.parent() {
                    let current_path =
                        std::env::var_os("PATH").unwrap_or_else(|| std::ffi::OsString::from(""));
                    let mut paths = std::env::split_paths(&current_path).collect::<Vec<_>>();
                    paths.insert(0, parent.to_path_buf());
                    let new_path = std::env::join_paths(paths)
                        .expect("failed to join PATH while configuring capnp");
                    std::env::set_var("PATH", &new_path);
                    println!(
                        "cargo:warning=Added {} to PATH for capnpc",
                        parent.display()
                    );
                }
            }

            capnpc::CompilerCommand::new()
                .src_prefix("../../schemas")
                .file("../../schemas/message.capnp")
                .file("../../schemas/identity.capnp")
                .file("../../schemas/presence.capnp")
                .file("../../schemas/friend.capnp")
                .file("../../schemas/voice.capnp")
                .file("../../schemas/account.capnp")
                .file("../../schemas/conversation.capnp")
                .run()
                .expect("Cap'n Proto schema compilation failed");

            // Set cfg flag so code can detect real codegen
            println!("cargo:rustc-cfg=capnp_codegen");
        }
    } else {
        panic!(
            "capnp binary not found. Install Cap'n Proto or use the project dev shell so schema codegen can run."
        );
    }

    // Re-run if schemas change
    println!("cargo:rerun-if-changed=../../schemas/");
}
