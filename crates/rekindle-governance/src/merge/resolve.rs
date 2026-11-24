//! Op-set preparation for `merge`: dedup, equivocation, causal completeness
//! and the ancestry every resolution rule asks about (plan E3.2).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use rekindle_types::governance::{GenesisCert, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::judge::Judge;
use crate::device_judge::{DeviceView, KeyJudgement, KeyRefusal};
use crate::op::VerifiedOp;

/// The op set `merge` resolves: complete (every dep present), in stamp
/// order (which is causal: a dep's lamport is below its dependant's), with
/// each op's causal past. An equivocating device key's ops from its fork on
/// stay in the set as ancestry only (`forked`, ADR 0022 rule 5): they resolve
/// their dependants' deps and are never applied.
pub(super) struct OpSet<'a> {
    pub ops: Vec<&'a VerifiedOp>,
    pub past: Ancestry,
    /// Per op: its device key forked at or below its seq (quarantined, kept
    /// as ancestry; ADR 0022 rule 5: the key, not the person).
    pub forked: Vec<bool>,
    /// Per forked op, its quarantine cause: the lowest-stamp other op of
    /// its device key at the fork seq outside its own causal past (the op a
    /// reader must hold to have seen the fork).
    pub fork_cause: Vec<Option<usize>>,
    /// Per op: why its device key is not effective for its author at its
    /// deps (`merge::judge`, ADR 0022 rule 6). Such an op stays in the set
    /// as ancestry only, its device elements harvested (rule 2), and is
    /// left out of every round.
    pub device_refused: Vec<Option<KeyRefusal>>,
    /// Per op: an own-key `DeviceReadmit` whose co-signer is not pre-fork
    /// (`prefork`, ADR 0022 rule 7, plan E3.2 K4b Q7): refused, ancestry
    /// only, never in the device view.
    pub readmit_refused: Vec<bool>,
    /// Ops whose deps are missing from the set (`pending_deps`), and ops
    /// whose device key is pending for their author at their deps (an
    /// unendorsed key is treated as a missing dep, rule 6), with every op
    /// that cites one.
    pub pending: Vec<OpHash>,
    /// Device keys that signed two different ops at one seq of their
    /// chain, each with its author and the lowest such seq (its fork).
    pub equivocators: Vec<(PseudonymKey, [u8; 32], u64)>,
    /// The device set the kept ops carry, harvested in stamp order (ADR
    /// 0022 rule 2: from every carrier, a forked or refused one included).
    pub devices: DeviceView,
}

/// `past.contains(i, j)`: op `j` is in op `i`'s causal past.
///
/// A dep is always earlier in stamp order than its dependant, so op `i`'s
/// past holds only ops below `i`: row `i` stores the bits for `0..i` alone
/// (a lower-triangular bit matrix). Rows are built in stamp order on the
/// first query that reaches them, so a merge pays only for the prefix of
/// the matrix its rules ask about (all of it once a late op is queried).
pub(super) struct Ancestry {
    /// Each op's deps, latest first, flattened: op `i`'s are
    /// `deps[dep_at[i]..dep_at[i + 1]]`.
    deps: Vec<usize>,
    dep_at: Vec<usize>,
    /// Where row `i`'s words start in `bits`; `row[i + 1]` is where it ends.
    row: Vec<usize>,
    /// Rows `0..built`, in order.
    bits: RefCell<Vec<u64>>,
    built: Cell<usize>,
}

impl Ancestry {
    pub(super) fn with_capacity(n: usize) -> Self {
        let mut row = Vec::with_capacity(n + 1);
        row.push(0);
        let mut dep_at = Vec::with_capacity(n + 1);
        dep_at.push(0);
        Self {
            deps: Vec::with_capacity(2 * n),
            dep_at,
            row,
            bits: RefCell::new(Vec::new()),
            built: Cell::new(0),
        }
    }

    /// Records the next op (in stamp order) and its deps.
    pub(super) fn push(&mut self, deps: &[usize]) {
        let i = self.dep_at.len() - 1;
        let from = self.deps.len();
        self.deps.extend_from_slice(deps);
        self.deps[from..].sort_unstable_by(|a, b| b.cmp(a));
        self.dep_at.push(self.deps.len());
        self.row.push(self.row[i] + i.div_ceil(64));
    }

    pub fn contains(&self, i: usize, j: usize) -> bool {
        tally!(contains, 1);
        if j >= i {
            return false;
        }
        if self.built.get() <= i {
            self.build_through(i);
        }
        self.bits.borrow()[self.row[i] + j / 64] & (1 << (j % 64)) != 0
    }

    /// Neither op is in the other's causal past.
    pub fn concurrent(&self, i: usize, j: usize) -> bool {
        i != j && !self.contains(i, j) && !self.contains(j, i)
    }

    /// Builds rows up to and including `last`. Each row is the union of
    /// its deps and their rows; a dep already in the row brings nothing
    /// new (its past is in the row with it), so the latest dep goes first
    /// and the rest are mostly skipped.
    fn build_through(&self, last: usize) {
        let mut bits = self.bits.borrow_mut();
        for i in self.built.get()..=last {
            tally!(rows_built, 1);
            bits.resize(self.row[i + 1], 0);
            let (head, row) = bits.split_at_mut(self.row[i]);
            for &d in &self.deps[self.dep_at[i]..self.dep_at[i + 1]] {
                if row[d / 64] & (1 << (d % 64)) == 0 {
                    tally!(words_ored, (self.row[d + 1] - self.row[d]) as u64);
                    for (t, f) in row.iter_mut().zip(&head[self.row[d]..self.row[d + 1]]) {
                        *t |= *f;
                    }
                    row[d / 64] |= 1 << (d % 64);
                }
            }
        }
        self.built.set(last + 1);
    }
}

/// An op hash as the key of `prepare`'s dep index, hashed by its first
/// eight bytes: the keys stored are hashes this reader computed (BLAKE3
/// over the op), so no writer can choose them, and the keyed SipHash of
/// eight bytes costs less than of thirty-two. Equality is the whole hash.
#[derive(PartialEq, Eq)]
struct IndexKey(OpHash);

impl Hash for IndexKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut lead = [0; 8];
        lead.copy_from_slice(&self.0 .0[..8]);
        state.write_u64(u64::from_le_bytes(lead));
    }
}

impl OpSet<'_> {
    /// Per op: left out of every round besides the gates' (its device key
    /// refused, `device_refused`, or an own-key readmit without a pre-fork
    /// co-signer, `readmit_refused`).
    pub(super) fn judged_out(&self) -> Vec<bool> {
        (self.device_refused.iter().zip(&self.readmit_refused))
            .map(|(d, r)| d.is_some() || *r)
            .collect()
    }
}

/// Prepare `ops` of the community `cert` pins (module doc). `admitted`:
/// the `DeviceReadmit`s a resolution applied (`merge::prepare_settled`);
/// any other readmit is kept as an op but never enters the device view,
/// since a readmit counts only once authorised (ADR 0022 rules 7 and 8,
/// plan E3.2 K4b BUG).
pub(super) fn prepare<'a>(
    cert: &GenesisCert,
    ops: &'a [VerifiedOp],
    admitted: &BTreeSet<OpHash>,
) -> OpSet<'a> {
    let mut sorted: Vec<&VerifiedOp> = ops.iter().collect();
    sorted.sort_by_key(|op| op.stamp());
    sorted.dedup_by_key(|op| op.hash());

    // Equivocation: two ops at one (device key, seq). A seq numbers its
    // device key's chain, so a member's two devices both write seq 0 and
    // neither forks: fork evidence is two different ops at one (device
    // key, seq), and it retires that key, not the person (ADR 0022 rule
    // 5). Every op of that key from the fork on is quarantined, on every
    // reader alike, and kept as ancestry.
    let fork_at = super::prefork::fork_seqs(&sorted);
    let mut index: HashMap<IndexKey, usize> = HashMap::with_capacity(sorted.len());
    let mut kept: Vec<&VerifiedOp> = Vec::with_capacity(sorted.len());
    let mut past = Ancestry::with_capacity(sorted.len());
    let mut deps: Vec<usize> = Vec::new();
    let mut pending = Vec::new();
    let mut forked = Vec::with_capacity(sorted.len());
    let mut device_refused = Vec::with_capacity(sorted.len());
    let mut devices = DeviceView::default();
    // The creator's first device is the cert's (S7d-keys.5), held before
    // any op is harvested, as `judge::member_view` holds it: the creator
    // joins by the cert, so without it the creator's own `DeviceKey` finds
    // no member record and its endorsement is dropped. Every op of the set
    // was verified for the one anchor.
    if let Some(op) = sorted.first() {
        devices.seed_genesis(op.anchor(), cert);
    }
    let mut judge = Judge::new(cert, &fork_at);
    // Per device key, its kept ops' indices in stamp order.
    let mut of_key: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
    // A forked key's author, from an op whose key is not judged pending
    // (an op whose key no one endorsed for its claimed author names no
    // equivocator).
    let mut author_of: HashMap<[u8; 32], PseudonymKey> = HashMap::new();
    for op in sorted {
        // An out-of-bounds or forked op stays in the DAG (validation
        // refuses the one, `forked` leaves the other out of every round),
        // so the ops that cite it are not held pending forever.
        deps.clear();
        let complete = op.op().deps.iter().all(|d| {
            index
                .get(&IndexKey(d.op_hash))
                .map(|&i| deps.push(i))
                .is_some()
        });
        let note_fork = |author_of: &mut HashMap<[u8; 32], PseudonymKey>| {
            if fork_at.contains_key(&op.op().device_key) {
                (author_of.entry(op.op().device_key)).or_insert_with(|| op.author().clone());
            }
        };
        if !complete {
            note_fork(&mut author_of);
            pending.push(op.hash());
            continue;
        }
        // Judged after `complete`: a key pending for its author is a
        // missing dep, never harvested (lead decision K4b Q5).
        let refused = match judge.judge(&kept, &past, &deps, op) {
            KeyJudgement::Effective => None,
            KeyJudgement::Pending(_) => {
                pending.push(op.hash());
                continue;
            }
            KeyJudgement::Refused(why) => Some(why),
        };
        note_fork(&mut author_of);
        let i = kept.len();
        index.insert(IndexKey(op.hash()), i);
        forked.push(
            fork_at
                .get(&op.op().device_key)
                .is_some_and(|seq| op.op().seq >= *seq),
        );
        device_refused.push(refused);
        past.push(&deps);
        // A carrier signed by the retired key itself lifts no floor: "the
        // key's own ops are left out, or a copy's own chain would lift the
        // floor over everything it wrote" (ADR 0022 rule 5, pass 3).
        let seen = retired_key(op)
            .filter(|key| *key != op.op().device_key)
            .and_then(|key| {
                (of_key.get(&key).into_iter().flatten())
                    .filter(|&&j| past.contains(i, j))
                    .map(|&j| kept[j].op().seq)
                    .max()
            });
        let unadmitted = is_readmit(op) && !admitted.contains(&op.hash());
        if !unadmitted {
            devices.harvest_seen(op, seen);
            judge.note(i, op, seen);
        }
        of_key.entry(op.op().device_key).or_default().push(i);
        kept.push(op);
    }
    retire_on_converged_set(
        &kept,
        &past,
        &devices,
        &of_key,
        &forked,
        &mut device_refused,
    );
    void_on_second_fork(&kept, &past, &fork_at, &forked, &mut device_refused);
    let fork_cause = fork_causes(&kept, &past, &forked, &fork_at);
    let readmit_refused =
        super::prefork::own_readmits_refused(cert, &kept, &past, &fork_at, &devices);
    OpSet {
        ops: kept,
        past,
        forked,
        fork_cause,
        device_refused,
        readmit_refused,
        pending,
        equivocators: (fork_at.into_iter())
            .filter_map(|(key, seq)| Some((author_of.get(&key)?.clone(), key, seq)))
            .collect(),
        devices,
    }
}

/// Whether `op` carries a `DeviceReadmit`.
pub(super) fn is_readmit(op: &VerifiedOp) -> bool {
    matches!(op.body(), GovernanceEntry::DeviceReadmit(_))
}

/// The key `op` retires, when it carries a `DeviceRetired`.
fn retired_key(op: &VerifiedOp) -> Option<[u8; 32]> {
    match op.body() {
        GovernanceEntry::DeviceRetired(r) => Some(r.device_key),
        _ => None,
    }
}

/// ADR 0022 rule 5 on the converged set (lead decision K4b Q2, Reading A):
/// a member's key is void above the lowest carrier floor of its
/// `DeviceRetired`s ([`crate::device_judge::MemberDevices::retirements`]),
/// raised to `W`, the key's highest seq in the strict causal past of an op
/// of another key that is effective at its deps, not forked, and carries
/// no `DeviceRetired` of the key (a carrier's past is its own floor). Its
/// ops above are refused `Retired`, quarantined and shown; an op the judge
/// at its deps refused `Retired` (its past held a retirement) at or below
/// moves back: `W` is the one reverse move ("exactly those move back to
/// applied, shown"). A pure function of the set, so order-free.
fn retire_on_converged_set(
    ops: &[&VerifiedOp],
    past: &Ancestry,
    devices: &DeviceView,
    of_key: &HashMap<[u8; 32], Vec<usize>>,
    forked: &[bool],
    device_refused: &mut [Option<KeyRefusal>],
) {
    let judged: Vec<bool> = device_refused.iter().map(Option::is_none).collect();
    for (member, md) in &devices.members {
        let keys: BTreeSet<[u8; 32]> = md.retirements.keys().map(|(k, _, _)| *k).collect();
        for key in keys {
            let (Some(from), Some(mine)) = (md.retired_from(&key), of_key.get(&key)) else {
                continue;
            };
            let witness = |j: usize| {
                (j + 1..ops.len()).any(|i| {
                    let o = ops[i];
                    o.op().device_key != key
                        && judged[i]
                        && !forked[i]
                        && retired_key(o) != Some(key)
                        && past.contains(i, j)
                })
            };
            let lifted = (mine.iter().rev())
                .filter(|&&j| ops[j].op().seq >= from)
                .find(|&&j| witness(j))
                .map(|&j| ops[j].op().seq.saturating_add(1));
            let from = lifted.map_or(from, |w| w.max(from));
            for &i in mine
                .iter()
                .filter(|&&i| ops[i].author() == member && !forked[i])
            {
                let retired = device_refused[i] == Some(KeyRefusal::Retired);
                if ops[i].op().seq >= from && device_refused[i].is_none() {
                    device_refused[i] = Some(KeyRefusal::Retired);
                } else if ops[i].op().seq < from && retired {
                    device_refused[i] = None;
                }
            }
        }
    }
}

/// ADR 0022 rule 7's second fork on the converged set (plan E3.2 K4b Q9):
/// fork evidence on a second distinct key of a member (order-free: fork
/// evidence on another key of the member exists; a `DeviceRetired` is
/// never a second fork) retires every key of that member above its floor,
/// the key's highest seq in the strict causal past of any op by another
/// member, a forked or refused one included, so nobody erases ops others
/// built on (rule 5's floor). A forked key is also void from its fork seq
/// (`forked`). A late op of another member raises a floor and so moves
/// back exactly the ops it cites, the one reverse move.
fn void_on_second_fork(
    ops: &[&VerifiedOp],
    past: &Ancestry,
    fork_at: &BTreeMap<[u8; 32], u64>,
    forked: &[bool],
    device_refused: &mut [Option<KeyRefusal>],
) {
    let mut forked_keys: BTreeMap<&PseudonymKey, BTreeSet<[u8; 32]>> = BTreeMap::new();
    for o in ops
        .iter()
        .filter(|o| fork_at.contains_key(&o.op().device_key))
    {
        forked_keys
            .entry(o.author())
            .or_default()
            .insert(o.op().device_key);
    }
    for (member, _) in forked_keys.iter().filter(|(_, keys)| keys.len() >= 2) {
        let mine: Vec<usize> = (0..ops.len())
            .filter(|&i| ops[i].author() == *member)
            .collect();
        let cited = |j: usize| {
            (j + 1..ops.len()).any(|i| ops[i].author() != *member && past.contains(i, j))
        };
        let mut floor: BTreeMap<[u8; 32], u64> = BTreeMap::new();
        for &j in mine.iter().filter(|&&j| cited(j)) {
            let at = floor.entry(ops[j].op().device_key).or_insert(0);
            *at = (*at).max(ops[j].op().seq);
        }
        for &i in &mine {
            let (key, seq) = (ops[i].op().device_key, ops[i].op().seq);
            let held = floor.get(&key).is_some_and(|f| seq <= *f);
            if !forked[i] && !held && device_refused[i].is_none() {
                device_refused[i] = Some(KeyRefusal::Retired);
            }
        }
    }
}

/// Each forked op's cause (`OpSet::fork_cause`): among its device key's ops at
/// the fork seq, the lowest in stamp order that is not the op and not in
/// its causal past; the lowest other one when its past holds them all.
fn fork_causes(
    ops: &[&VerifiedOp],
    past: &Ancestry,
    forked: &[bool],
    fork_at: &BTreeMap<[u8; 32], u64>,
) -> Vec<Option<usize>> {
    let at_fork: Vec<usize> = (0..ops.len())
        .filter(|&j| fork_at.get(&ops[j].op().device_key) == Some(&ops[j].op().seq))
        .collect();
    (0..ops.len())
        .map(|i| {
            if !forked[i] {
                return None;
            }
            let siblings = || {
                (at_fork.iter().copied())
                    .filter(move |&j| j != i && ops[j].op().device_key == ops[i].op().device_key)
            };
            siblings()
                .find(|&j| !past.contains(i, j))
                .or_else(|| siblings().next())
        })
        .collect()
}

#[cfg(test)]
#[path = "retire_tests.rs"]
mod retire_tests;

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rekindle_types::id::PseudonymKey;

    use super::prepare;
    use crate::fixture::{meta, op};
    use crate::op::VerifiedOp;

    /// A DAG of `n` ops, op `k` citing the earlier ops `picks` selects.
    fn dag(n: usize, picks: &[u64]) -> Vec<VerifiedOp> {
        let mut ops: Vec<VerifiedOp> = Vec::new();
        for k in 0..n {
            let bits = picks[k % picks.len()].rotate_left(u32::try_from(k % 64).unwrap_or(0));
            let deps: Vec<&VerifiedOp> = (0..k)
                .filter(|&d| bits & (1 << ((k + 3 * d) % 64)) != 0 && k - d <= 70)
                .map(|d| &ops[d])
                .collect();
            let author = PseudonymKey([u8::try_from(k % 251).unwrap_or(0); 32]);
            let next = op(
                &author,
                u64::try_from(k).unwrap_or(0),
                &deps,
                k as u64,
                meta("m"),
            );
            ops.push(next);
        }
        ops
    }

    proptest! {
        /// R4-5: rows built on demand, in any query order, with the
        /// contained-dep skip, equal causal reachability over the deps.
        #[test]
        fn ancestry_is_reachability_in_any_query_order(
            n in 1usize..160,
            picks in proptest::collection::vec(any::<u64>(), 1..8),
            order in proptest::collection::vec(any::<u32>(), 1..400),
        ) {
            let ops = dag(n, &picks);
            let creator = PseudonymKey([0; 32]);
            let cert = crate::fixture::cert(&creator, rekindle_types::governance::AdmissionMode::Open);
            let set = prepare(&cert, &ops, &std::collections::BTreeSet::new());
            let at = |h| set.ops.iter().position(|o| o.hash() == h);
            let mut reach = vec![vec![false; set.ops.len()]; set.ops.len()];
            for i in 0..set.ops.len() {
                for d in &set.ops[i].op().deps {
                    let d = at(d.op_hash).unwrap_or(usize::MAX);
                    prop_assert!(d < i);
                    reach[i][d] = true;
                    let below = reach[d].clone();
                    for (r, b) in reach[i].iter_mut().zip(below) {
                        *r |= b;
                    }
                }
            }
            let m = set.ops.len();
            for q in order {
                let q = q as usize;
                let (i, j) = (q % m, (q / m) % m);
                prop_assert_eq!(set.past.contains(i, j), reach[i][j], "contains({}, {})", i, j);
            }
        }
    }
}
