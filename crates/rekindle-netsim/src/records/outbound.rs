//! A node's outbound watch state (plan VU2 item 4), after Veilid 0.5.8
//! (`~/veilid-wt-058/veilid-core/src/storage_manager`):
//!
//! - **the clock:** every timer runs on the node's own wall clock, read
//!   non-decreasing (`Timestamp::now_non_decreasing`), so a wall clock jump
//!   (a desktop sleep) makes a silent watch due at once;
//! - **the fallback inspection** is a silence timer per watch: due once the
//!   current interval has passed since the latest of establishment, the
//!   last fallback inspection and the last push the node received
//!   (`outbound_watch_state.rs:347-372`, a push touches it at
//!   `watch_value.rs:1931-1936`). The interval starts at
//!   [`FALLBACK_INTERVAL`], doubles on each inspection that finds nothing
//!   up to [`MAX_INSPECT_INTERVAL`] (`:141-146`,
//!   `note_change_inspect_outcome`, `watch_value.rs:2368-2397`) and resets
//!   on a productive inspection, a parameter change or a newly holding node
//!   (`watch_value.rs:1305-1315`); a dead value-changed route resets it
//!   with this watch's phase of the base interval (`:147-160`,
//!   `watch_value.rs:961-972`). At the ceiling, a watch at consensus that
//!   heard a push since its last inspection waits for its renewal window
//!   (`:374-402`). An unestablished watch is inspected every
//!   [`MAX_INSPECT_INTERVAL`] (`outbound_watch.rs:100-104`);
//! - **report marks:** a subkey reported to the app is marked unread
//!   (`:51-55`). A value-carrying report marks with no copies owed
//!   (`:268-273`); a range-only report reports only the subkeys not already
//!   marked and counts a copy on the rest (`:251-266`,
//!   `report_value_change_hint`, `watch_value.rs:1993-2050`), sending
//!   nothing when none is news. Only a get that reaches the network clears
//!   a mark (`get_value.rs:196-197`, `consume_watch_read_subkeys`,
//!   `watch_value.rs:2275-2296`); a local read leaves it. A cleared mark
//!   with copies owed queues one re-hint, sent as a range-only report on
//!   the next check tick (`tasks/check_outbound_watches.rs:97-103`,
//!   `send_pending_rehints`, `watch_value.rs:2298-2320`). Change-inspection
//!   reports bypass the marks (`report_change_inspect`,
//!   `watch_value.rs:1523-1578`).
//!
//! Not modelled: the spread after a Degraded period (the netsim has no
//! Degraded state; its Offline return sweeps every watch,
//! [`super::catchup`]), the survival stretch under measured loss
//! (`outbound_watch_state.rs:175-193`) and the forced reconcile a
//! productive inspection asks for (`watch_value.rs:1560-1566`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use rekindle_records::port::ValueChange;

use super::replicas::GET_COUNT;
use super::watches::MAX_WATCH_EXPIRATION;
use crate::network::Inner;

/// `WATCH_FALLBACK_INSPECT_INTERVAL` (`outbound_watch_state.rs:7`).
pub(crate) const FALLBACK_INTERVAL: Duration = Duration::from_secs(30);
/// `max_watch_inspect_interval_ms` (`veilid_config.rs:1018`,
/// `storage_manager/mod.rs:201-208`).
pub(crate) const MAX_INSPECT_INTERVAL: Duration = Duration::from_secs(300);
/// `renew_outbound_watches`: 3 × `get_value_timeout_ms` 10 s
/// (`storage_manager/mod.rs:194-195`, `constants.rs:24`,
/// `veilid_config.rs:1007`).
pub(crate) const RENEW_WINDOW: Duration = Duration::from_secs(30);

pub(crate) fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// A node's outbound watch manager: its queued and running change
/// inspections, each watch's state and its owed re-hints.
#[derive(Debug)]
pub(crate) struct Outbound {
    /// Queued change inspections: record key → not before (node clock µs)
    /// (`needs_change_inspection`).
    pub queue: HashMap<String, u64>,
    /// Running inspections: record key → when its answer is in (node clock
    /// µs) (`change_inspects_in_flight`).
    pub in_flight: HashMap<String, u64>,
    /// Consecutive failed inspections per record (`outbound_watch.rs`
    /// `note_change_inspect_failure`).
    pub failures: HashMap<String, u32>,
    /// Each established watch's state.
    pub states: HashMap<String, WatchState>,
    /// When each unestablished watch was last inspected (node clock µs).
    pub unestablished: HashMap<String, u64>,
    /// Owed re-hints by record (`pending_rehints`).
    pub rehints: BTreeMap<String, BTreeSet<u32>>,
    /// Whether the node's DHT was online at the last watch tick.
    pub was_online: bool,
    /// The last node clock reading (µs), so it never runs backwards.
    clock: u64,
}

impl Default for Outbound {
    fn default() -> Self {
        Self {
            queue: HashMap::new(),
            in_flight: HashMap::new(),
            failures: HashMap::new(),
            states: HashMap::new(),
            unestablished: HashMap::new(),
            rehints: BTreeMap::new(),
            was_online: true,
            clock: 0,
        }
    }
}

/// One watch's `OutboundWatchState` (the fields the netsim drives).
#[derive(Debug, Clone)]
pub(crate) struct WatchState {
    established: u64,
    /// When the node's watch was last renewed (node clock µs): its nodes'
    /// earliest expiry is this plus [`MAX_WATCH_EXPIRATION`].
    renewed: u64,
    pub last_push: Option<u64>,
    pub last_fallback: Option<u64>,
    interval: Option<u64>,
    not_before: Option<u64>,
    /// Unread reported subkeys, with the range-only copies suppressed.
    pub unread: BTreeMap<u32, u32>,
}

impl WatchState {
    fn new(at: u64) -> Self {
        Self {
            established: at,
            renewed: at,
            last_push: None,
            last_fallback: None,
            interval: None,
            not_before: None,
            unread: BTreeMap::new(),
        }
    }

    /// `fallback_inspect_interval` (`outbound_watch_state.rs:413-418`).
    pub(crate) fn interval(&self) -> u64 {
        self.interval
            .unwrap_or(micros(FALLBACK_INTERVAL))
            .min(micros(MAX_INSPECT_INTERVAL))
    }

    /// `fallback_reference_ts` (`:362-367`).
    fn reference(&self) -> u64 {
        [self.last_fallback, self.last_push]
            .into_iter()
            .flatten()
            .fold(self.established, u64::max)
    }

    /// `needs_fallback_inspect` (`:349-360`).
    fn needs(&self, at: u64) -> bool {
        if self.not_before.is_some_and(|t| at < t) {
            return false;
        }
        at >= self.reference().saturating_add(self.interval())
    }

    /// `pushed_since_fallback` (`:404-410`).
    fn pushed_since_fallback(&self) -> bool {
        let since = self.last_fallback.unwrap_or(self.established);
        self.last_push.is_some_and(|t| t >= since)
    }

    /// `fallback_inspect_due` (`:374-402`), with the renewal window over
    /// the node's own renewal (the netsim renews on every tick it reaches a
    /// replica, Veilid only inside the window: `renewed` keeps Veilid's).
    fn due(&self, at: u64, held: usize) -> bool {
        if !self.needs(at) {
            return false;
        }
        if held < GET_COUNT
            || self.interval() < micros(MAX_INSPECT_INTERVAL)
            || !self.pushed_since_fallback()
        {
            return true;
        }
        self.renewal_due(at)
    }

    fn renewal_due(&self, at: u64) -> bool {
        at.saturating_add(micros(RENEW_WINDOW))
            >= self.renewed.saturating_add(micros(MAX_WATCH_EXPIRATION))
    }

    /// `back_off_fallback_inspect` (`:141-146`).
    fn back_off(&mut self) {
        let doubled = self.interval().saturating_mul(2);
        self.interval = Some(doubled.min(micros(MAX_INSPECT_INTERVAL)));
    }

    /// `reset_fallback_inspect_backoff` (`:147-160`).
    fn reset(&mut self, spread_from: Option<u64>) {
        self.interval = None;
        self.not_before = spread_from.map(|at| {
            let reference = self.reference();
            let base = micros(FALLBACK_INTERVAL);
            let periods = at.saturating_sub(reference).div_ceil(base);
            reference.saturating_add(periods.saturating_mul(base))
        });
    }

    /// `mark_hinted_subkeys` (`:251-266`): the news among `subkeys`.
    fn mark_hinted(&mut self, subkeys: &BTreeSet<u32>) -> BTreeSet<u32> {
        let mut news = BTreeSet::new();
        for s in subkeys {
            if let Some(copies) = self.unread.get_mut(s) {
                *copies = copies.saturating_add(1);
            } else {
                self.unread.insert(*s, 0);
                news.insert(*s);
            }
        }
        news
    }
}

/// The node's wall clock now (µs): true time plus its skew, never behind
/// its last reading.
pub(crate) fn clock(inner: &mut Inner, node: usize, now: u64) -> u64 {
    let slot = &mut inner.nodes[node];
    let wall = now.saturating_add_signed(slot.skew_ms.saturating_mul(1000));
    slot.outbound.clock = slot.outbound.clock.max(wall);
    slot.outbound.clock
}

/// Whether `node`'s process is suspended ([`crate::Fault::Sleep`]).
pub(crate) fn asleep(inner: &Inner, node: usize) -> bool {
    inner.faults.asleep.contains(&node)
}

/// A replica newly holds `node`'s watch on `key`: the state starts at its
/// first establishment, and a later newcomer resets the backoff
/// (`watch_value.rs:1305-1315`).
pub(crate) fn established(inner: &mut Inner, node: usize, key: &str, now: u64) {
    let at = clock(inner, node, now);
    let out = &mut inner.nodes[node].outbound;
    out.unestablished.remove(key);
    out.states
        .entry(key.to_string())
        .and_modify(|s| s.reset(None))
        .or_insert_with(|| WatchState::new(at));
}

/// The watch's parameters changed: reset its backoff (`watch_value.rs:1310-1315`).
pub(crate) fn params_changed(inner: &mut Inner, node: usize, key: &str) {
    if let Some(s) = inner.nodes[node].outbound.states.get_mut(key) {
        s.reset(None);
    }
}

/// The watch's value-changed route died: reset with this watch's phase
/// (`watch_value.rs:961-972`).
pub(crate) fn route_lost(inner: &mut Inner, node: usize, key: &str, now: u64) {
    let at = clock(inner, node, now);
    if let Some(s) = inner.nodes[node].outbound.states.get_mut(key) {
        s.reset(Some(at));
    }
}

/// The watch ended: its state, marks and queued work go with it.
pub(crate) fn ended(inner: &mut Inner, node: usize, key: &str) {
    let out = &mut inner.nodes[node].outbound;
    out.states.remove(key);
    out.unestablished.remove(key);
    out.rehints.remove(key);
    out.queue.remove(key);
    out.failures.remove(key);
}

/// A push of `key` reached `node` (`watch_value.rs:1931-1936`).
pub(crate) fn pushed(inner: &mut Inner, node: usize, key: &str, now: u64) {
    let at = clock(inner, node, now);
    if let Some(s) = inner.nodes[node].outbound.states.get_mut(key) {
        s.last_push = Some(at);
    }
}

/// A value-carrying report of `subkeys` (`mark_value_reported_subkeys`).
pub(crate) fn value_reported(inner: &mut Inner, node: usize, key: &str, subkeys: &BTreeSet<u32>) {
    if let Some(s) = inner.nodes[node].outbound.states.get_mut(key) {
        for k in subkeys {
            s.unread.insert(*k, 0);
        }
    }
}

/// A range-only report of `subkeys`: the news among them (all of them for
/// a watch with no state yet, as Veilid reports when it has none).
pub(crate) fn hinted(
    inner: &mut Inner,
    node: usize,
    key: &str,
    subkeys: &BTreeSet<u32>,
) -> BTreeSet<u32> {
    match inner.nodes[node].outbound.states.get_mut(key) {
        Some(s) => s.mark_hinted(subkeys),
        None => subkeys.clone(),
    }
}

/// `node` read `key`/`subkey` from the network: its mark is consumed, and
/// a suppressed copy owes one re-hint (`watch_value.rs:2275-2296`).
pub(crate) fn read_back(inner: &mut Inner, node: usize, key: &str, subkey: u32) {
    let out = &mut inner.nodes[node].outbound;
    let Some(s) = out.states.get_mut(key) else {
        return;
    };
    if s.unread.remove(&subkey).is_some_and(|copies| copies > 0) {
        out.rehints
            .entry(key.to_string())
            .or_default()
            .insert(subkey);
    }
}

/// An inspection answered: clear its failures and back off or reset
/// (`note_change_inspect_outcome`, `watch_value.rs:2368-2397`).
pub(crate) fn inspected(inner: &mut Inner, node: usize, key: &str, productive: bool) {
    let out = &mut inner.nodes[node].outbound;
    out.failures.remove(key);
    if let Some(s) = out.states.get_mut(key) {
        if productive {
            s.reset(None);
        } else {
            s.back_off();
        }
    }
}

/// The node's watches whose fallback inspection is due at `at`, each
/// stamped as inspected (`check_fallback_inspections_inner`,
/// `tasks/check_outbound_watches.rs:8-68`); a watch inside its renewal
/// window renews.
pub(crate) fn fallbacks_due(inner: &mut Inner, node: usize, at: u64) -> Vec<String> {
    let mut keys: Vec<String> = inner.nodes[node]
        .watches
        .iter()
        .filter(|(_, d)| d.is_some())
        .map(|(k, _)| k.clone())
        .collect();
    keys.sort();
    let mut due = Vec::new();
    for key in keys {
        let held = super::watches::held(inner, node, &key);
        let out = &mut inner.nodes[node].outbound;
        if let Some(s) = out.states.get_mut(&key) {
            if s.due(at, held) {
                s.last_fallback = Some(at);
                s.not_before = None;
                due.push(key.clone());
            }
            if s.renewal_due(at) && held > 0 {
                s.renewed = at;
            }
        } else {
            let last = out.unestablished.get(&key).copied();
            if last.is_none_or(|t| at >= t.saturating_add(micros(MAX_INSPECT_INTERVAL))) {
                out.unestablished.insert(key.clone(), at);
                due.push(key);
            }
        }
    }
    due
}

/// Send the node's owed re-hints as range-only reports
/// (`send_pending_rehints`, `watch_value.rs:2298-2320`).
pub(crate) fn take_rehints(inner: &mut Inner, node: usize) -> Vec<ValueChange> {
    let owed = std::mem::take(&mut inner.nodes[node].outbound.rehints);
    owed.into_iter()
        .filter_map(|(key, subkeys)| hint(inner, node, &key, &subkeys))
        .collect()
}

/// `report_value_change_hint` with no value (`watch_value.rs:1993-2050`):
/// trimmed to the watched subkeys, the news reported and the watch counted
/// down; nothing when none is news.
fn hint(inner: &mut Inner, node: usize, key: &str, subkeys: &BTreeSet<u32>) -> Option<ValueChange> {
    let watched = inner.nodes[node]
        .watches
        .get(key)?
        .as_ref()?
        .subkeys
        .clone();
    let reportable: BTreeSet<u32> = subkeys.intersection(&watched).copied().collect();
    if reportable.is_empty() {
        return None;
    }
    let news = hinted(inner, node, key, &reportable);
    if news.is_empty() {
        return None;
    }
    let desired = inner.nodes[node].watches.get_mut(key)?.as_mut()?;
    desired.count = desired.count.saturating_sub(1);
    Some(ValueChange {
        key: key.to_string(),
        subkeys: news.into_iter().collect(),
        count: desired.count,
        value: None,
    })
}
