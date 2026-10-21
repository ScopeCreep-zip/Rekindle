//! A record's replicas on the simulated DHT (plan E2.B slice S4, M6), after
//! the pinned veilid-core (`~/veilid/veilid-core/src`):
//!
//! - every record lives on [`REPLICAS`] stores, Veilid's `consensus_width`
//!   (the closest nodes to the key a fanout counts, `veilid_config.rs:916`,
//!   `rpc_processor/fanout/fanout_call.rs:292-330`);
//! - **placement:** replica `r` sits beside simulated node
//!   `(offset + r) % n`, where `n` is the node count when the record is
//!   created and `offset` folds the record key's text (the key comes from
//!   the creator's seeded generator, so one seed places every replica the
//!   same way and no extra draw shifts the run's random stream). A node
//!   reaches a replica unless a partition separates it from the replica's
//!   node, so each side of a partition reaches the replicas beside its own
//!   nodes; a node added later has no replica of an earlier record beside
//!   it. A replica is a DHT store, not that node's storage: the node going
//!   offline does not take it down;
//! - a replica keeps the first value it receives at a seq: an older or a
//!   same-seq different value is answered with what it holds, the same
//!   value is a success (`storage_manager/set_value.rs:700-719`);
//! - a set fans out in placement order over the reachable replicas and is
//!   done at [`SET_COUNT`] accepts (`set_value_count`,
//!   `veilid_config.rs:914`). A replica answering a newer value, or a
//!   same-seq different one while the fanout still carries the writer's own
//!   value, restarts the fanout on that value and sends it again to the
//!   replicas that had accepted (`set_value.rs:470-494`, `AcceptedNewerRestart`
//!   and `all_accepted_to_queued`, `fanout_call.rs:438-442`). A same-seq
//!   conflict after a restart is stale and does not restart again (the
//!   model's bound on Veilid's conflict loop);
//! - a get starts from the local value and takes a reachable replica's value
//!   only when its seq is newer; a same-seq different value is stale
//!   (`get_value.rs:275-276,405-426`); it is done at [`GET_COUNT`] matching
//!   replicas (`get_value_count`, `veilid_config.rs:911`) and otherwise
//!   answers the newest it saw (`get_value.rs:538-567`).

use std::collections::{HashSet, VecDeque};

use rekindle_records::port::RecordValue;

use crate::network::pair;

/// Replicas per record: Veilid's `consensus_width` (`veilid_config.rs:916`).
pub(crate) const REPLICAS: usize = 10;
/// Accepts a set needs: Veilid's `set_value_count` (`veilid_config.rs:914`).
pub(crate) const SET_COUNT: usize = 5;
/// Matching replicas a get stops at: `get_value_count` (`veilid_config.rs:911`).
pub(crate) const GET_COUNT: usize = 3;

/// One replica's store of a record.
#[derive(Debug, Clone)]
pub(crate) struct Replica {
    /// The simulated node this replica sits beside (for partitions).
    pub home: usize,
    /// The DHT store holding this replica (`home` is `store % nodes`):
    /// each simulated node is beside [`REPLICAS`] stores, so the records of
    /// one store are bounded ([`super::stores`]).
    pub store: usize,
    /// When the store last used this record (its LRU stamp).
    pub used: u64,
    pub values: Vec<Option<RecordValue>>,
    /// The watches this replica holds ([`super::watches`]).
    pub watches: Vec<super::watches::ReplicaWatch>,
}

/// The replicas of a new record with `subkeys` subkeys, placed among
/// `nodes` nodes from `key` (see the module notes).
pub(crate) fn place(key: &str, nodes: usize, subkeys: usize) -> Vec<Replica> {
    let offset = key.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(usize::from(b))
    });
    let stores = nodes.max(1) * REPLICAS;
    (0..REPLICAS)
        .map(|r| Replica {
            home: offset.wrapping_add(r) % nodes.max(1),
            store: offset.wrapping_add(r) % stores,
            used: 0,
            values: vec![None; subkeys],
            watches: Vec::new(),
        })
        .collect()
}

/// Whether `node` reaches a replica beside `home`. `usize::MAX` is an
/// outsider that reaches every replica (a scenario's modified client).
pub(crate) fn reaches(partitions: &HashSet<(usize, usize)>, node: usize, home: usize) -> bool {
    node == usize::MAX || node == home || !partitions.contains(&pair(node, home))
}

/// The replicas `node` reaches, in placement order.
pub(crate) fn reachable(
    replicas: &[Replica],
    partitions: &HashSet<(usize, usize)>,
    node: usize,
) -> Vec<usize> {
    replicas
        .iter()
        .enumerate()
        .filter(|(_, r)| reaches(partitions, node, r.home))
        .map(|(i, _)| i)
        .collect()
}

/// A replica's answer to a set (`set_value.rs:700-719`).
enum Offer {
    Stored,
    Same,
    Ignored(RecordValue),
}

fn offer(slot: &mut Option<RecordValue>, value: &RecordValue) -> Offer {
    match slot {
        Some(held) if held == value => Offer::Same,
        Some(held) if held.seq >= value.seq => Offer::Ignored(held.clone()),
        _ => {
            *slot = Some(value.clone());
            Offer::Stored
        }
    }
}

/// What a set's fanout did.
pub(crate) struct SetFanout {
    /// The value the fanout ended on: the writer's own, or the newer one a
    /// replica restarted it on.
    pub value: RecordValue,
    /// Whether [`SET_COUNT`] replicas accepted `value`.
    pub consensus: bool,
    /// How many replicas accepted `value`.
    pub accepted: usize,
    /// The replicas that stored a value in this fanout, in order (one may
    /// appear twice: a restart sent it the newer value).
    pub stored: Vec<usize>,
}

/// Fan `ours` out to the `reach` replicas at `subkey`.
pub(crate) fn set(
    replicas: &mut [Replica],
    reach: &[usize],
    subkey: usize,
    ours: &RecordValue,
) -> SetFanout {
    let mut ctx = ours.clone();
    let mut queue: VecDeque<usize> = reach.iter().copied().collect();
    let mut accepted: Vec<usize> = Vec::new();
    let mut stored = Vec::new();
    while accepted.len() < SET_COUNT {
        let Some(r) = queue.pop_front() else {
            break;
        };
        let Some(slot) = replicas.get_mut(r).and_then(|x| x.values.get_mut(subkey)) else {
            continue;
        };
        match offer(slot, &ctx) {
            Offer::Stored => {
                stored.push(r);
                accepted.push(r);
            }
            Offer::Same => accepted.push(r),
            Offer::Ignored(held) => {
                if held.seq > ctx.seq || ctx == *ours {
                    ctx = held;
                    for a in accepted.drain(..).rev() {
                        queue.push_front(a);
                    }
                    accepted.push(r);
                }
            }
        }
    }
    SetFanout {
        consensus: accepted.len() >= SET_COUNT,
        accepted: accepted.len(),
        value: ctx,
        stored,
    }
}

/// A get's answer at `subkey` over the `reach` replicas, from `local`.
pub(crate) fn get(
    replicas: &[Replica],
    reach: &[usize],
    subkey: usize,
    local: Option<&RecordValue>,
) -> Option<RecordValue> {
    let mut ctx = local.cloned();
    let mut matching = 0;
    for r in reach {
        let Some(held) = replicas
            .get(*r)
            .and_then(|x| x.values.get(subkey)?.as_ref())
        else {
            continue;
        };
        match &ctx {
            Some(c) if held == c => matching += 1,
            Some(c) if held.seq <= c.seq => {}
            _ => {
                ctx = Some(held.clone());
                matching = 1;
            }
        }
        if matching >= GET_COUNT {
            break;
        }
    }
    ctx
}

/// The newest seq the `reach` replicas hold at `subkey` (an inspect's
/// network seq).
pub(crate) fn newest_seq(replicas: &[Replica], reach: &[usize], subkey: usize) -> Option<u32> {
    reach
        .iter()
        .filter_map(|r| {
            replicas
                .get(*r)?
                .values
                .get(subkey)?
                .as_ref()
                .map(|v| v.seq)
        })
        .max()
}

/// Whether any of the `reach` replicas holds a value at any of `subkeys`
/// (all when `None`): the record's descriptor travels with its values.
pub(crate) fn holds(replicas: &[Replica], reach: &[usize], subkeys: Option<&[u32]>) -> bool {
    reach
        .iter()
        .filter_map(|r| replicas.get(*r))
        .any(|r| match subkeys {
            Some(s) => s
                .iter()
                .any(|s| r.values.get(*s as usize).is_some_and(Option::is_some)),
            None => r.values.iter().any(Option::is_some),
        })
}
