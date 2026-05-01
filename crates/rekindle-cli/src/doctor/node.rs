//! Doctor checks: node health — attachment, route, uptime.

use crate::doctor::{Check, Status};
use crate::helpers;
use crate::transport::TransportHandle;

/// Run all node health checks.
pub fn checks(handle: &TransportHandle) -> Vec<Check> {
    let snapshot = handle.node().status_snapshot();
    let mut results = Vec::new();

    // node.running — is the transport node attached to the network
    results.push(Check {
        id: "node.running".into(),
        category: "node",
        status: if snapshot.is_attached {
            Status::Pass
        } else {
            Status::Fail
        },
        value: snapshot.attachment.clone(),
        description: if snapshot.is_attached {
            String::new()
        } else {
            "node is not attached — check network connectivity\n\
             restart with: rekindle node restart"
                .into()
        },
    });

    // node.public_internet — is public internet reachable via the node
    results.push(Check {
        id: "node.public_internet".into(),
        category: "node",
        status: if snapshot.public_internet_ready {
            Status::Pass
        } else if snapshot.is_attached {
            Status::Warn
        } else {
            Status::Fail
        },
        value: if snapshot.public_internet_ready {
            "ready".into()
        } else {
            "not ready".into()
        },
        description: if snapshot.public_internet_ready {
            String::new()
        } else {
            "public internet not yet ready — DHT and peer discovery may be limited\n\
             wait for full attachment or check firewall settings"
                .into()
        },
    });

    // node.route.allocated — is a private route allocated for receiving messages
    results.push(Check {
        id: "node.route.allocated".into(),
        category: "node",
        status: if snapshot.route_allocated {
            Status::Pass
        } else {
            Status::Fail
        },
        value: if snapshot.route_allocated {
            match snapshot.route_age_secs {
                Some(age) => format!("active ({})", helpers::format_uptime(age)),
                None => "active".into(),
            }
        } else {
            "not allocated".into()
        },
        description: if snapshot.route_allocated {
            String::new()
        } else {
            "no private route — peers cannot reach you\n\
             allocate with: rekindle network routes --refresh"
                .into()
        },
    });

    // node.uptime — how long the node has been running
    let uptime_str = helpers::format_uptime(snapshot.uptime_secs);
    results.push(Check {
        id: "node.uptime".into(),
        category: "node",
        status: if snapshot.uptime_secs > 10 {
            Status::Pass
        } else {
            Status::Warn
        },
        value: uptime_str,
        description: if snapshot.uptime_secs <= 10 {
            "node just started — network may not be fully warmed up yet".into()
        } else {
            String::new()
        },
    });

    results
}
