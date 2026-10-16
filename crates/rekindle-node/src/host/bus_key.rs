//! The bus server's Noise static keypair: generated at each start and held
//! in memory; only its public half is published (`bus.pub`), for clients
//! to connect with (plan D1). A restart therefore rotates it.

use rekindle_ipc::ZeroizingKeypair;

/// Files an earlier daemon kept the bus private key in. A secret left on
/// disk is removed at start.
const STALE_SECRET_FILES: [&str; 2] = ["bus.key", "bus.checksum"];

/// Generate this run's bus keypair and publish its public key in
/// `runtime_dir`.
///
/// # Errors
/// Generation fails, a stale secret cannot be removed, or `bus.pub`
/// cannot be written.
pub async fn generate_and_publish(
    runtime_dir: &std::path::Path,
) -> anyhow::Result<ZeroizingKeypair> {
    for name in STALE_SECRET_FILES {
        match tokio::fs::remove_file(runtime_dir.join(name)).await {
            Ok(()) => tracing::info!(file = name, "removed a bus key file an earlier daemon left"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(anyhow::anyhow!("cannot remove stale {name}: {e}")),
        }
    }
    let kp = rekindle_ipc::generate_keypair()?;
    let public: [u8; 32] = kp
        .public()
        .try_into()
        .map_err(|_| anyhow::anyhow!("the bus public key is not 32 bytes"))?;
    rekindle_ipc::noise_keys::write_bus_public_key(&public).await?;
    tracing::info!("bus keypair generated for this run");
    Ok(kp)
}
