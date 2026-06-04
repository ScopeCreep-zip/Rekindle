# Presence

`rekindle-presence` (Phase 21) hosts the presence primitives plus
four orchestrators that compose them: friend presence, community
presence, idle detection, and game presence. The crate is a Tier-5
pure-logic surface parameterised over `PresenceDeps` traits; the
Tauri-side wiring lives in `src-tauri/src/services/presence_adapter/`.

## The two flavours of presence

Rekindle has two presence systems because they have different
threat models and different update cadences.

**Friend presence** is a one-to-one signal between trusted peers.
Each friend publishes their status (online / away / busy / offline /
invisible) to a DHT subkey on their profile record. Watchers
subscribe via `watch_dht_values` and receive `ValueChange` updates
within seconds. Friends already know each other's identity keys, so
there is no anonymity concern.

**Community presence** is many-to-many among unlinkable pseudonyms.
Members write their per-community presence under their pseudonym to
the member registry's per-slot DHT key. Watchers poll the registry
(steady ticks every 30 s, rapid ticks every 5 s during onboarding
or rebuild) rather than watching every slot — the watch budget would
not scale to 255-member communities × N communities.

`rekindle-presence::friend` and `rekindle-presence::community` are
the orchestrators for each flavour. They share the lower-tier
primitives in `status.rs`, `heartbeat.rs`, `idle.rs`, and
`poll_buckets.rs`.

## Public surface

The crate's `lib.rs` re-exports cover the integration surface:

| Re-export | Surface |
|---|---|
| `presence_poll_tick`, `presence_poll_tick_public`, `start_presence_poll`, `steady_poll_duration` | Community presence poll loop |
| `compute_rebuild_plan`, `GossipOverlayPlan`, `GossipOverlaySnapshot`, `GossipRebuildOutcome` | Gossip overlay rebuild decisions |
| `compute_merged_roles`, `role_ids_from_governance` | Effective-role computation against `GovernanceState` |
| `aggregate_event_rsvps`, `EventRsvpEntry` | Event RSVP aggregation across slots |
| `compute_profile_diff`, `ProfileDiffOutcome`, `MemberProfileSnapshot` | Per-community profile diff (theme color, bio, avatar) |
| `parse_and_classify_row`, `ClassifiedRow`, `DiscoveredRow`, `persist_discovered_registry_members` | Member-registry row parsing and persistence |
| `presence_event_id_bytes`, `write_our_presence`, `PresenceWrite` | Outbound presence writes |
| `random_peer_sample`, `gossip_degree` | Peer-selection helpers |
| `run_initial_sync` | Bootstrap sync on join |

Plus timing constants:

| Constant | Value | Purpose |
|---|---|---|
| `STEADY_TICK_INTERVAL_SECS` | 30 | Default community presence poll cadence |
| `RAPID_TICK_INTERVAL_SECS` | 5 | Faster cadence during onboarding / rebuild |
| `RAPID_TICKS` | small N | Number of rapid ticks before falling back to steady |
| `STALE_HEARTBEAT_SECS` | configurable | Threshold after which a member is treated as offline |
| `STALE_SYNC_RETRY_SECS` | configurable | Cooldown before retrying a sync that returned stale data |
| `MAX_SYNC_ATTEMPTS` | configurable | Per-bootstrap sync attempt budget |
| `SUBKEYS_PER_SEGMENT` | 255 | Universal SMPL slot count |

## Friend presence

Friend presence is event-driven, not polled. The flow is:

1. On login, `services/presence_service.rs` calls
   `node.watch_dht_values(profile_key, subkey=2..=5)` for every
   friend. The watch covers status, status message, route blob, and
   game info.
2. Veilid emits `VeilidUpdate::ValueChange` on subkey writes.
3. The dispatch loop forwards the change to `presence_service`,
   which decodes the new value and updates `AppState.friends`.
4. `event_dispatch::emit_journaled` fires a `PresenceEvent`
   (`StatusChanged` / `GameChanged` / `FriendOnline` / `FriendOffline`)
   through the standard event router.

When a friend's watch fails (Veilid drops it under route churn), the
friend's key is added to `AppState.unwatched_friends`. The
`sync_service` background loop polls those keys with
`force_refresh=true` until the watch is re-armed.

## Community presence

Community presence is poll-based on a two-rate scheduler:

- **Steady tick** (default 30 s) — the long-term background cadence.
  Reads each segment's member registry, classifies rows by
  `parse_and_classify_row`, persists discovered members, computes
  the gossip overlay rebuild plan via `compute_rebuild_plan`, and
  emits any `MemberPresenceChanged` / `MembersRefreshed` events.
- **Rapid tick** (5 s × `RAPID_TICKS` count) — fired immediately
  after a join, an admin-triggered rebuild, or a `SegmentAdded`
  governance entry. Falls back to the steady tick once the rapid
  budget is exhausted.

The two-rate scheduler avoids burning watch budget on large
communities while keeping onboarding feel "live." See
`presence_poll_tick` and `start_presence_poll` for the orchestration
loop.

`compute_rebuild_plan` decides whether the existing gossip overlay
covers the latest member set or whether peers need to re-sample with
`random_peer_sample(gossip_degree(N))`. The decision is pure: it
takes a snapshot of the current overlay and the merged governance
state and returns a `GossipOverlayPlan` (no peer count changes / add
M peers / replace overlay entirely).

Profile diffs are computed per-tick via `compute_profile_diff`,
which compares the persisted `MemberProfileSnapshot` against the
freshly fetched per-community profile fields (theme color, bio,
avatar reference, banner reference). The diff outcome drives
`CommunityEvent::MemberPresenceChanged` payload composition.

## Idle and game

`rekindle-presence::idle` exposes the auto-away decision logic:
given the wall clock, the last-activity timestamp, the user's
configured idle threshold, and the current status, decide whether
to transition to `Away` and what status to restore on activity. The
Tauri-side runtime in `services/idle_service.rs` polls Tauri's idle
events and feeds them into the decision function.

`rekindle-presence::heartbeat` issues the periodic profile
re-publish (subkey 6 route blob) that keeps mailbox-fallback fresh
when the profile watch lapses.

Game presence is wired through `services/game_publisher.rs`: the
`rekindle-game-detect` scanner produces a `GameStatus`, the
publisher writes it to profile subkey 4, and friends with active
watches see the change as a `GameChanged` event.

## Adapter (`services/presence_adapter/`)

The adapter implements `rekindle_presence::PresenceDeps` against the
live AppState and AppHandle. It is split into the standard
runtime / adapter / pure-logic categories (see
[`services-pattern.md`](services-pattern.md)) — `deps_impl.rs`
delegates per-method to `state_reads.rs`, `state_mutations.rs`,
`dht.rs`, `persist.rs`, and `events.rs`.

## Why the split exists

Before Phase 21, presence logic lived in a single
`services/community/presence/{poll, registry, sync}.rs` directory
with all decisions baked into the Tauri runtime. The harvest pulled
the decisions out (everything that does not need `AppState`) and
kept the wiring in. The crate's pure-logic surfaces are
property-tested independently, which the previous arrangement made
impractical.

The next phase to land in this area is a friend-presence overhaul
that gives the friend flavour the same orchestrator shape (today
`friend.rs` is still mostly a thin wrapper; `friend_sync.rs` carries
the heavier logic). The current code structure does not block that
work — both flavours already live in one crate.
