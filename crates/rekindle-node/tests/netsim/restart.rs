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

use crate::harness::{node_span, temp_root, SimNode};

/// `Host::start` with `TransportSource::Ports` on `node`'s ports, over the
/// data root in `dir`, handing its shell stages to `shell`.
pub async fn host_on(
    node: Node,
    dir: &tempfile::TempDir,
    shell: Option<Arc<dyn ShellHooks>>,
) -> Arc<Host> {
    let root = temp_root(dir);
    let storage_dir = root.data.join("veilid").display().to_string();
    let index = node.index;
    Host::start(HostConfig {
        root: claim(root).expect("claim the data root"),
        stores: HostStores::default(),
        transport: TransportSource::Ports {
            config: rekindle_types::config::TransportConfig {
                storage_dir,
                ..Default::default()
            },
            ports: node.ports,
            events: node.events,
        },
        serve_bus: false,
        clock: Arc::clone(&node.clock) as Arc<dyn rekindle_utils::Clock>,
        rng: Arc::clone(&node.rng) as Arc<dyn rekindle_utils::RandomSource>,
        idle: Arc::clone(&node.idle) as Arc<dyn rekindle_utils::IdleSource>,
        shell,
    })
    .instrument(node_span(index))
    .await
    .expect("host starts")
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
            ..
        } = self;
        host.shutdown().await.expect("the host stops");
        drop(host);
        sim.terminate_node(index);
        tokio::time::sleep(down).await;
        let node = sim.start_node(index);
        let routes = Arc::clone(&node.ports.routes);
        let host = host_on(node, &dir, None).await;
        sim.quiesce().await;
        Self {
            index,
            host,
            public_key,
            routes,
            dir,
        }
    }
}
