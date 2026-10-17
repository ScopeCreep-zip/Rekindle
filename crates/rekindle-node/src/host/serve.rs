//! Serving the IPC bus (plan D3.1): the bus server, its accept loop and the
//! host's own subscriber connection, which receives every request.

use std::sync::Arc;

use super::runtime::Bus;
use super::{bus_key, watchdog, ConfigFault};
use crate::daemon::dispatch::DaemonContext;
use crate::daemon::shutdown::ExitReason;

/// Bind the bus, hand it the journal, start its accept loop, and connect the
/// host's own subscriber (requests are unicast to it).
pub(super) async fn serve(ctx: &Arc<DaemonContext>) -> anyhow::Result<Bus> {
    let runtime_dir = rekindle_ipc::runtime_dir()?;
    tokio::fs::create_dir_all(&runtime_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let bus_keypair = bus_key::generate_and_publish(&runtime_dir)
        .await
        .map_err(ConfigFault)?;

    // The subscriber's keypair is registered before the bind, so the
    // server authenticates it at Internal when it connects.
    let subscriber_kp = rekindle_ipc::generate_keypair()
        .map_err(|e| anyhow::anyhow!("host subscriber keypair generation failed: {e}"))?;
    let subscriber_pub: [u8; 32] = subscriber_kp
        .as_inner()
        .public
        .clone()
        .try_into()
        .map_err(|_| anyhow::anyhow!("subscriber pubkey is not 32 bytes"))?;
    ctx.registry.write().await.register(
        rekindle_ipc::server::DAEMON_AGENT_NAME.to_string(),
        subscriber_pub,
        rekindle_ipc::SecurityLevel::Internal,
        rekindle_ipc::AgentType::System,
        // `Internal`: not scoped by agent capabilities.
        Vec::new(),
    );

    let socket_path = rekindle_ipc::socket_path()?;
    let server = Arc::new(rekindle_ipc::BusServer::bind(
        &socket_path,
        bus_keypair.into_inner(),
        Arc::clone(&ctx.registry),
    )?);
    tracing::info!(path = %socket_path.display(), "IPC bus server bound");
    server.start_event_delivery(Arc::clone(&ctx.journal));

    let accept = spawn_accept(&server, &ctx.shutdown);
    let subscriber = spawn_bus_subscriber(Arc::clone(ctx), socket_path, subscriber_kp);
    Ok(Bus {
        server,
        accept,
        subscriber: tokio::sync::Mutex::new(Some(subscriber)),
    })
}

/// Run the bus server's accept loop for the life of the host; a fatal
/// accept error stops the host. `Host::shutdown` stops and joins it.
fn spawn_accept(
    server: &Arc<rekindle_ipc::BusServer>,
    shutdown: &Arc<crate::daemon::shutdown::Shutdown>,
) -> tokio::task::JoinHandle<()> {
    let (server, shutdown) = (Arc::clone(server), Arc::clone(shutdown));
    tokio::spawn(async move {
        if let Err(e) = server.run().await {
            tracing::error!(error = %e, "bus server fatal error");
            shutdown.request(ExitReason::Requested);
        }
    })
}

/// Spawn the host's own bus subscriber: it connects to its own socket as a
/// privileged internal agent with the keypair registered in the clearance
/// registry, and the server unicasts requests to it. It sends READY=1 once
/// connected: before that the bus answers every request 503.
fn spawn_bus_subscriber(
    ctx: Arc<DaemonContext>,
    socket: std::path::PathBuf,
    keypair: rekindle_ipc::ZeroizingKeypair,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let server_pub = match rekindle_ipc::noise_keys::read_bus_public_key().await {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(error = %e, "host subscriber: cannot read bus public key");
                ctx.shutdown.request(ExitReason::SubscriberFailed);
                return;
            }
        };
        let client = match rekindle_ipc::BusClient::connect_with_retry(
            uuid::Uuid::now_v7(),
            &socket,
            &server_pub,
            keypair.as_inner(),
            5,
            std::time::Duration::from_millis(200),
        )
        .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "host subscriber: failed to connect to own socket");
                ctx.shutdown.request(ExitReason::SubscriberFailed);
                return;
            }
        };
        tracing::info!("host bus subscriber connected");
        watchdog::notify_ready();
        ctx.run_subscriber(client).await;
    })
}
