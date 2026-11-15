//! A record's replicas on the simulated DHT (plan E2.B slice S4, M6; plan
//! VU2 items 2 and 9), after veilid-core 0.5.8 (Rekindle's fork, ADR 0017;
//! `veilid-core/src`):
//!
//! - every record lives on [`REPLICAS`] stores, Veilid's `consensus_width`
//!   (`veilid_config.rs:1013`: the closest nodes to the key a fanout counts,
//!   less those that time out or disqualify, `fanout_call.rs:401-441`);
//! - **placement:** replica `r` sits beside simulated node
//!   `(offset + r) % n`, where `n` is the node count when the record is
//!   created and `offset` folds the record key's text (the key comes from
//!   the creator's seeded generator, so one seed places every replica the
//!   same way and no extra draw shifts the run's random stream). A node
//!   reaches a replica unless a partition separates it from the replica's
//!   node, so each side of a partition reaches the replicas beside its own
//!   nodes; a node added later has no replica of an earlier record beside
//!   it. A replica is a DHT store, not that node's storage: the node going
//!   offline does not take it down. Replica index order is closeness to the
//!   key, the order a [`Tally`] breaks ties by;
//! - **a value is its ciphertext** ([`Cipher`]): every record is encrypted
//!   (`storage_manager/create_record.rs:93`) under a fresh random nonce per
//!   encryption (`record_encryption.rs:43`), and Veilid compares the
//!   encrypted value data, nonce included (`value_tally.rs:84-88`,
//!   `set_value.rs:836`). Two encryptions of one value at one seq are
//!   rivals; a resend of the local copy keeps its bytes
//!   (`set_value.rs:353-366`);
//! - a set's fanout, and a replica's answer to it, are [`super::fanout`];
//! - a get starts from the local value, which only sets the floor (no
//!   node's answer stands behind it), takes a newer seq at once, and at the
//!   newest seq settles a split by the tally (`get_value.rs:44-90`); it is
//!   done at [`GET_COUNT`] holders of its value (`get_value_count`,
//!   `veilid_config.rs:1008`). The settled value replaces the local copy
//!   when its seq is newer, or at the same seq only on a forced refresh
//!   with no write of ours waiting; otherwise the get returns the local copy
//!   (`get_value.rs:600-659`).

use std::collections::HashSet;

use rekindle_records::port::{RecordSchema, RecordValue};

use crate::network::{pair, Inner};

/// Replicas per record: Veilid's `consensus_width` (`veilid_config.rs:1013`).
pub(crate) const REPLICAS: usize = 10;
/// Accepts a set needs: Veilid's `set_value_count` (`veilid_config.rs:1011`),
/// used by [`super::fanout`].
pub(crate) const SET_COUNT: usize = 5;
/// Holders a get stops at, and the holders a set that ends without
/// consensus still lands at: `get_value_count` (`veilid_config.rs:1008`,
/// `storage_manager/mod.rs:990-1030`).
pub(crate) const GET_COUNT: usize = 3;

/// A value as a store holds it: the value and the nonce of the encryption
/// that made its bytes (see the module notes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cipher {
    pub value: RecordValue,
    pub nonce: u64,
}

/// Which replicas hold which value at one seq (`value_tally.rs:13-107`):
/// the value more replicas hold wins, and as many holders go to the one
/// held closer to the key (the lower replica index).
#[derive(Default)]
pub(crate) struct Tally {
    holders: Vec<(Cipher, Vec<usize>)>,
}

impl Tally {
    /// Forget every holder, for a newer seq.
    pub(crate) fn clear(&mut self) {
        self.holders.clear();
    }

    /// Record that replica `r` holds `value`.
    pub(crate) fn add(&mut self, value: &Cipher, r: usize) {
        match self.holders.iter_mut().find(|(v, _)| v == value) {
            Some((_, rs)) if rs.contains(&r) => {}
            Some((_, rs)) => rs.push(r),
            None => self.holders.push((value.clone(), vec![r])),
        }
    }

    fn of(&self, value: &Cipher) -> Option<&[usize]> {
        self.holders
            .iter()
            .find(|(v, _)| v == value)
            .map(|(_, rs)| rs.as_slice())
    }

    /// How many replicas hold `value`.
    pub(crate) fn holders(&self, value: &Cipher) -> usize {
        self.of(value).map_or(0, <[usize]>::len)
    }

    /// Whether `rival` beats `current`: more holders, or as many with a
    /// closer closest holder; a `current` nobody holds loses to any rival
    /// (`value_tally.rs:90-106`).
    pub(crate) fn beats(&self, rival: &Cipher, current: &Cipher) -> bool {
        let Some(current) = self.of(current) else {
            return true;
        };
        let rival = self.of(rival).unwrap_or(&[]);
        match rival.len().cmp(&current.len()) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => rival.iter().min() < current.iter().min(),
        }
    }

    /// Whether some replica holds another value than `current`
    /// (`value_tally.rs:52-57`).
    pub(crate) fn conflicted(&self, current: &Cipher) -> bool {
        self.holders.iter().any(|(v, _)| v != current)
    }
}

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
    /// The nonce of each value this replica took from a fanout, with that
    /// value; a value set any other way (a scenario's injected write) has
    /// nonce 0 ([`Replica::cipher`]).
    pub nonces: Vec<Option<(RecordValue, u64)>>,
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
            nonces: vec![None; subkeys],
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

impl Replica {
    /// The value this replica holds at `subkey`, as its ciphertext.
    pub(crate) fn cipher(&self, subkey: usize) -> Option<Cipher> {
        let value = self.values.get(subkey)?.clone()?;
        let nonce = match self.nonces.get(subkey) {
            Some(Some((v, nonce))) if *v == value => *nonce,
            _ => 0,
        };
        Some(Cipher { value, nonce })
    }

    /// Store `value` at `subkey`.
    pub(crate) fn put(&mut self, subkey: usize, value: &Cipher) {
        if let (Some(slot), Some(nonce)) =
            (self.values.get_mut(subkey), self.nonces.get_mut(subkey))
        {
            *slot = Some(value.value.clone());
            *nonce = Some((value.value.clone(), value.nonce));
        }
    }
}

/// A get's settled value at `subkey` over the `reach` replicas, from
/// `local` (see the module notes).
pub(crate) fn settle(
    schema: &RecordSchema,
    replicas: &[Replica],
    reach: &[usize],
    subkey: usize,
    local: Option<&Cipher>,
) -> Option<Cipher> {
    let mut current = local.cloned();
    let mut tally = Tally::default();
    for r in reach {
        let Some(held) = replicas
            .get(*r)
            .and_then(|x| x.cipher(subkey))
            // An answer past the size rule is invalid (`get_value.rs:438-470`).
            .filter(|v| super::size::fits(schema, &v.value))
        else {
            continue;
        };
        tally.add(&held, *r);
        match &current {
            Some(c) if held.value.seq < c.value.seq => {}
            Some(c) if held.value.seq == c.value.seq => {
                if held != *c && tally.beats(&held, c) {
                    current = Some(held);
                }
            }
            _ => {
                tally.clear();
                tally.add(&held, *r);
                current = Some(held);
            }
        }
        if current
            .as_ref()
            .is_some_and(|c| tally.holders(c) >= GET_COUNT)
        {
            break;
        }
    }
    current
}

/// A get's settled value at `subkey` over the `reach` replicas, from a
/// `local` copy given as its value: its bytes are those of the closest
/// replica holding it, if any.
pub(crate) fn get(
    schema: &RecordSchema,
    replicas: &[Replica],
    reach: &[usize],
    subkey: usize,
    local: Option<&RecordValue>,
) -> Option<RecordValue> {
    let local = local.map(|v| Cipher {
        value: v.clone(),
        nonce: nonce_of(replicas, subkey, v),
    });
    settle(schema, replicas, reach, subkey, local.as_ref()).map(|c| c.value)
}

/// The nonce of the closest replica holding `value` at `subkey`, else 0.
fn nonce_of(replicas: &[Replica], subkey: usize, value: &RecordValue) -> u64 {
    replicas
        .iter()
        .filter_map(|x| x.cipher(subkey))
        .find(|c| c.value == *value)
        .map_or(0, |c| c.nonce)
}

/// `node`'s local copy of `key`'s `subkey` as its ciphertext. A copy taken
/// by a path that does not carry the nonce (a watch's value, a catch-up
/// get) has the bytes of the closest replica holding it.
pub(crate) fn local(inner: &Inner, node: usize, key: &str, subkey: u32) -> Option<Cipher> {
    let at = (key.to_string(), subkey);
    let slot = &inner.nodes[node];
    let value = slot.local.get(&at)?.clone();
    let nonce = match slot.nonces.get(&at) {
        Some((v, nonce)) if *v == value => *nonce,
        _ => inner
            .records
            .get(key)
            .map_or(0, |r| nonce_of(&r.replicas, subkey as usize, &value)),
    };
    Some(Cipher { value, nonce })
}

/// Make `value` `node`'s local copy of `key`'s `subkey`.
pub(crate) fn keep(inner: &mut Inner, node: usize, key: &str, subkey: u32, value: &Cipher) {
    let at = (key.to_string(), subkey);
    let slot = &mut inner.nodes[node];
    slot.local.insert(at.clone(), value.value.clone());
    slot.nonces.insert(at, (value.value.clone(), value.nonce));
}

/// A fresh encryption of `value` (`record_encryption.rs:43`).
pub(crate) fn encrypt(inner: &mut Inner, value: RecordValue) -> Cipher {
    inner.last_nonce += 1;
    Cipher {
        value,
        nonce: inner.last_nonce,
    }
}

/// A get over the network by `node`: the settled value of the replicas it
/// reaches, stored locally when it replaces the local copy; the local copy
/// otherwise (see the module notes). `force` is a forced refresh.
pub(crate) fn refresh(
    inner: &mut Inner,
    node: usize,
    key: &str,
    subkey: u32,
    force: bool,
) -> Option<RecordValue> {
    let local = local(inner, node, key, subkey);
    let reach = inner.records.get(key).map_or_else(Vec::new, |r| {
        reachable(&r.replicas, &inner.faults.partitions, node)
    });
    super::stores::touch(inner, key, &reach);
    // A get that reached the network consumes its report mark
    // (0.5.8 `get_value.rs:196-197`, [`super::outbound`]).
    super::outbound::read_back(inner, node, key, subkey);
    let found = inner.records.get(key).and_then(|r| {
        settle(
            &r.schema,
            &r.replicas,
            &reach,
            subkey as usize,
            local.as_ref(),
        )
    })?;
    let replaces = local.as_ref().is_none_or(|l| {
        if found.value.seq != l.value.seq {
            return found.value.seq > l.value.seq;
        }
        // `get_result_replaces_local` (`get_value.rs:636-659`).
        force
            && found != *l
            && !inner.nodes[node]
                .pushes
                .contains_key(&(key.to_string(), subkey))
    });
    if replaces {
        keep(inner, node, key, subkey, &found);
        Some(found.value)
    } else {
        local.map(|l| l.value)
    }
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
