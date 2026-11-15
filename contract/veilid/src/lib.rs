//! Mock Veilid nodes for the contract tests (plan VU decision 6).
//!
//! Every node here is veilid-core's `mock-api`: one in-process simulation
//! per `api_startup`, all of them on one process-global network
//! (`veilid_api/mock/mod.rs:118-130`), keyed by program name plus namespace
//! (`mod.rs:26-38`), so each node takes a name of its own. The mock keeps
//! nothing on disk (veilid-core `Cargo.toml` `mock-api`), and its records,
//! routes and watches are shared, so two nodes here talk to each other.
//!
//! The nodes are started with `rekindle-protocol`'s own config builder and
//! update mapping, so what a test sees is what the real adapters produce.

use std::sync::Arc;
use std::time::Duration;

use rekindle_protocol::node::NodeConfig;
use rekindle_protocol::ports::to_port_event;
use rekindle_protocol::veilid_config::build_veilid_config;
use rekindle_records::port::PortEvent;
use rekindle_types::config::SafetyProfile;
use tokio::sync::mpsc;
use veilid_core::VeilidAPI;

/// How long a test waits for an update the mock delivers synchronously.
pub const EVENT_WAIT: Duration = Duration::from_secs(5);

/// A node's config: its own namespace, and a storage directory the mock
/// never opens.
pub fn node_config(name: &str) -> NodeConfig {
    NodeConfig {
        storage_dir: std::env::temp_dir()
            .join("rekindle-veilid-contract")
            .join(name)
            .to_string_lossy()
            .into_owned(),
        app_namespace: format!("rekindle-contract-{name}"),
        qualifier: "rekindle-contract".into(),
        veilid: rekindle_types::config::VeilidStartupOptions::default(),
    }
}

/// A started and attached mock node whose updates arrive through
/// [`to_port_event`], as `VeilidPorts::start` installs it; `None` is an
/// update the mapping drops, so a test sees drops as well as events.
pub struct MockNode {
    pub api: VeilidAPI,
    pub events: mpsc::UnboundedReceiver<Option<PortEvent>>,
}

/// Start a mock node named `name` and attach it, as `rekindle-protocol`'s
/// `startup` does (`node.rs:133-156`), keeping the API handle so a test
/// can drive the mock directly.
pub async fn start_node(name: &str) -> MockNode {
    let (tx, events) = mpsc::unbounded_channel();
    let config = node_config(name);
    let api = veilid_core::api_startup(
        Arc::new(move |update| {
            let _ = tx.send(to_port_event(update));
        }),
        build_veilid_config(
            &config.app_namespace,
            &config.qualifier,
            &config.storage_dir,
            &config.veilid,
        ),
    )
    .await
    .expect("mock api_startup");
    api.attach().await.expect("mock attach");
    let mut node = MockNode { api, events };
    // The attach's own update (mock `api.rs:409-420`), so a test's first
    // event is the one it drove.
    assert!(matches!(
        node.next_outcome().await,
        Some(PortEvent::Attachment {
            is_attached: true,
            ..
        })
    ));
    node
}

impl MockNode {
    /// The next mapped event, skipping updates the mapping dropped.
    pub async fn next_event(&mut self) -> PortEvent {
        next_event(&mut self.events).await
    }

    /// The next raw outcome of the mapping: `Some(event)` or a drop.
    pub async fn next_outcome(&mut self) -> Option<PortEvent> {
        tokio::time::timeout(EVENT_WAIT, self.events.recv())
            .await
            .expect("an update within the wait")
            .expect("the update channel is open")
    }
}

/// The next mapped event on a channel of mapping outcomes.
pub async fn next_event(events: &mut mpsc::UnboundedReceiver<Option<PortEvent>>) -> PortEvent {
    loop {
        let outcome = tokio::time::timeout(EVENT_WAIT, events.recv())
            .await
            .expect("an event within the wait")
            .expect("the update channel is open");
        if let Some(event) = outcome {
            return event;
        }
    }
}

/// The DHT safety profile the tests open records with.
pub fn profile() -> SafetyProfile {
    SafetyProfile::default_text()
}
