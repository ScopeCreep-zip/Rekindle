use std::path::Path;

const SCHEMAS: &[&str] = &[
    "message", "identity", "presence", "community", "friend", "voice",
    "account", "conversation",
];

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();

    // Cap'n Proto schema compilation — requires `capnp` CLI tool.
    // Install via: brew install capnp (macOS), apt install capnproto (Linux), nix-shell -p capnproto
    if let Ok(output) = std::process::Command::new("capnp").arg("--version").output() {
        let version = String::from_utf8_lossy(&output.stdout);
        println!("cargo:warning=Using capnp: {}", version.trim());

        capnpc::CompilerCommand::new()
            .src_prefix("../../schemas")
            .file("../../schemas/message.capnp")
            .file("../../schemas/identity.capnp")
            .file("../../schemas/presence.capnp")
            .file("../../schemas/community.capnp")
            .file("../../schemas/friend.capnp")
            .file("../../schemas/voice.capnp")
            .file("../../schemas/account.capnp")
            .file("../../schemas/conversation.capnp")
            .run()
            .expect("Cap'n Proto schema compilation failed");

        // Set cfg flag so code can detect real codegen
        println!("cargo:rustc-cfg=capnp_codegen");
    } else {
        println!("cargo:warning=capnp binary not found — generating stub modules.");
        println!("cargo:warning=Install capnproto for real serialization support.");

        // Create empty stub files so include!() doesn't fail
        for schema in SCHEMAS {
            let path = Path::new(&out_dir).join(format!("{schema}_capnp.rs"));
            if !path.exists() {
                std::fs::write(
                    &path,
                    "// stub — install capnp to generate real bindings\n",
                )
                .expect("failed to write stub file");
            }
        }
    }

    // Re-run if schemas change
    println!("cargo:rerun-if-changed=../../schemas/");
}
