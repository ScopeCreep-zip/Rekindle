//! Doctor checks: network health — peers, routes, DHT reachability.

use crate::doctor::{Check, Status};
use crate::transport::TransportHandle;

/// Run all network health checks.
pub async fn checks(handle: &TransportHandle) -> Vec<Check> {
    let mut results = Vec::new();
    let snapshot = handle.node().status_snapshot();
    let circuit = handle.node().peers().read().circuit_summary();

    // network.peers.cached — total known peers with valid routes
    results.push(Check {
        id: "network.peers.cached".into(),
        category: "network",
        status: if circuit.healthy > 0 || !snapshot.is_attached {
            Status::Pass
        } else if circuit.total > 0 {
            Status::Warn
        } else {
            // No peers at all — might be first boot or network isolated
            if snapshot.is_attached {
                Status::Warn
            } else {
                Status::Pass // Not attached yet, no peers expected
            }
        },
        value: format!("{} peers", circuit.total),
        description: if circuit.total == 0 && snapshot.is_attached {
            "no known peers — the network may be bootstrapping\n\
             join a community to discover peers: rekindle community join"
                .into()
        } else {
            String::new()
        },
    });

    // network.peers.reachable — healthy peers vs total
    if circuit.total > 0 {
        #[allow(clippy::cast_precision_loss)]
        let reachable_ratio =
            circuit.healthy as f64 / circuit.total.max(1) as f64;
        results.push(Check {
            id: "network.peers.reachable".into(),
            category: "network",
            status: if reachable_ratio >= 0.5 {
                Status::Pass
            } else if reachable_ratio > 0.0 {
                Status::Warn
            } else {
                Status::Fail
            },
            value: format!(
                "{}/{} reachable ({} degraded, {} circuit open)",
                circuit.healthy, circuit.total, circuit.degraded, circuit.circuit_open
            ),
            description: if circuit.circuit_open > 0 {
                format!(
                    "{} peers have tripped circuit breakers — they may be offline or routes stale\n\
                     refresh routes: rekindle network routes --refresh",
                    circuit.circuit_open
                )
            } else if circuit.degraded > 0 {
                format!(
                    "{} peers have stale routes — they may need re-import\n\
                     check peers: rekindle network peers",
                    circuit.degraded
                )
            } else {
                String::new()
            },
        });
    }

    // network.dht.profile — can we read our own profile DHT record
    let profile_check = match handle.node().dht() {
        Ok(dht) => {
            // Try to read our profile display name subkey as a liveness test
            match dht
                .profile()
                .get_subkey(
                    // We'd need the profile key from the session, but doctor checks
                    // don't have session access in the network module. We check DHT
                    // availability instead.
                    "__self_test__",
                    0,
                )
                .await
            {
                // A "record not open" error means DHT is functional but the test key
                // doesn't exist, which is expected. The important thing is that the
                // DHT operation didn't fail with NotStarted or NetworkNotReady.
                Ok(_) => Check {
                    id: "network.dht.available".into(),
                    category: "network",
                    status: Status::Pass,
                    value: "accessible".into(),
                    description: String::new(),
                },
                Err(rekindle_transport::TransportError::RecordNotOpen { .. }) => Check {
                    id: "network.dht.available".into(),
                    category: "network",
                    status: Status::Pass,
                    value: "accessible (record not open — expected for test probe)".into(),
                    description: String::new(),
                },
                Err(rekindle_transport::TransportError::NotStarted) => Check {
                    id: "network.dht.available".into(),
                    category: "network",
                    status: Status::Fail,
                    value: "node not started".into(),
                    description: "start the node: rekindle node start".into(),
                },
                Err(e) => Check {
                    id: "network.dht.available".into(),
                    category: "network",
                    status: Status::Warn,
                    value: format!("error: {e}"),
                    description: "DHT may be temporarily unavailable".into(),
                },
            }
        }
        Err(e) => Check {
            id: "network.dht.available".into(),
            category: "network",
            status: Status::Fail,
            value: format!("unavailable: {e}"),
            description: "DHT not accessible — node may not be fully attached".into(),
        },
    };
    results.push(profile_check);

    results
}
