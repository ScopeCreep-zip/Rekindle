//! The bus server's Noise static keypair: loaded with tamper detection, or
//! generated on first start.

use rekindle_ipc::ZeroizingKeypair;

/// The files that make up the bus keypair.
const BUS_KEY_FILES: [&str; 3] = ["bus.pub", "bus.key", "bus.checksum"];

/// Load the bus server keypair from `runtime_dir`, or generate and persist
/// one on first start.
///
/// First start means none of the keypair's files exist. Any other state —
/// a file missing, a bad length, a checksum mismatch — is an error: the
/// daemon refuses to start rather than silently replace a key that may have
/// been tampered with.
///
/// # Errors
/// An incomplete, malformed or tampered keypair, or a generation or
/// persistence failure.
pub async fn load_or_generate(runtime_dir: &std::path::Path) -> anyhow::Result<ZeroizingKeypair> {
    let first_start = BUS_KEY_FILES
        .iter()
        .all(|name| !runtime_dir.join(name).exists());
    if !first_start {
        let kp = rekindle_ipc::noise_keys::read_bus_keypair().await?;
        tracing::info!("loaded existing bus keypair");
        return Ok(kp);
    }

    let kp = rekindle_ipc::generate_keypair()?;
    rekindle_ipc::noise_keys::write_bus_keypair(kp.as_inner()).await?;
    tracing::info!("bus keypair generated");
    Ok(kp)
}
