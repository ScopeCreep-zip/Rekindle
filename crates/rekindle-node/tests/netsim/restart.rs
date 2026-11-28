//! A node's Host started on its data root, and a node restart (plan E3.2
//! S7, NR1): the Host shuts down, the node's Veilid terminates and starts
//! again over the same data (`rekindle_netsim`'s `restart` module, after
//! veilid-core 0.5.8 `storage_manager/mod.rs:649-695`), and a new Host starts
//! on the same data root through the production startup path
//! (`Host::start`, `claim`), locked as any fresh start.

use std::sync::Arc;
use std::time::Duration;

use rekindle_netsim::{Netsim, Node};
use rekindle_node::host::{claim, Host, HostConfig, HostStores, ShellHooks, TransportSource};
use tracing::Instrument;

use crate::harness::{node_span, SimNode};
use crate::shell_node::{BareShell, ShellInbound, SimMode};

/// `Host::start` on `node`'s ports over the data root in `dir`, handing its
/// shell stages to `shell`. A `Daemon` node's Host starts its transport on
/// the ports (`TransportSource::Ports`, the daemon's `Start`); a `Shell`
/// node's transport is built first and adopted (`TransportSource::Adopted`),
/// as the desktop's (`src-tauri/src/setup.rs`), with a hold-nothing shell
/// when `shell` is `None`.
pub async fn host_on(
    node: Node,
    dir: &tempfile::TempDir,
    shell: Option<Arc<dyn ShellHooks>>,
    mode: SimMode,
) -> (Arc<Host>, Arc<rekindle_db::DbHandle>) {
    let root = rekindle_db::paths::DataRoot::under(dir.path());
    let storage_dir = root.data.join("veilid").display().to_string();
    let index = node.index;
    let config = rekindle_types::config::TransportConfig {
        storage_dir,
        ..Default::default()
    };
    let stores = HostStores::default();
    let db = Arc::clone(&stores.db);
    let rng = Arc::clone(&node.rng) as Arc<dyn rekindle_utils::RandomSource>;
    let clock = Arc::clone(&node.clock) as Arc<dyn rekindle_utils::Clock>;
    let (transport, inbound, shell) = match mode {
        SimMode::Daemon => (
            TransportSource::Ports {
                config,
                ports: node.ports,
                events: node.events,
            },
            None,
            shell,
        ),
        SimMode::Shell => {
            let inbound = Arc::new(ShellInbound::new(
                Arc::clone(&clock),
                Arc::clone(&stores.session),
            ));
            let transport = rekindle_transport::TransportNode::from_ports(
                config,
                node.ports,
                node.events,
                Arc::clone(&inbound),
                Arc::clone(&stores.session),
                Arc::clone(&rng),
            );
            let shell =
                shell.unwrap_or_else(|| Arc::new(BareShell::default()) as Arc<dyn ShellHooks>);
            (
                TransportSource::Adopted(Arc::new(transport)),
                Some(inbound),
                Some(shell),
            )
        }
    };
    let host = Host::start(HostConfig {
        root: claim(root).expect("claim the data root"),
        stores,
        transport,
        serve_bus: false,
        clock,
        rng,
        idle: Arc::clone(&node.idle) as Arc<dyn rekindle_utils::IdleSource>,
        server_query: std::sync::Arc::new(rekindle_game_detect::server_query::UdpServerQuery),
        shell,
    })
    .instrument(node_span(index))
    .await
    .expect("host starts");
    if let Some(inbound) = inbound {
        inbound.attach(&host);
    }
    (host, db)
}

impl SimNode {
    /// Restart the node: its Host exits and lets go of the data root, its
    /// process is down for `down` (the other nodes see it detached), then
    /// it starts again over the same data root, locked.
    pub async fn restart(self, sim: &Netsim, down: Duration) -> Self {
        let Self {
            index,
            host,
            public_key,
            dir,
            mode,
            ..
        } = self;
        host.shutdown().await.expect("the host stops");
        drop(host);
        sim.terminate_node(index);
        tokio::time::sleep(down).await;
        let node = sim.start_node(index);
        let routes = Arc::clone(&node.ports.routes);
        let (host, db) = host_on(node, &dir, None, mode).await;
        sim.quiesce().await;
        Self {
            index,
            host,
            db,
            public_key,
            routes,
            dir,
            mode,
        }
    }
}
