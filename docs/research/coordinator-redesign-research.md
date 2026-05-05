# Coordinator Redesign Research

**Date:** 2026-03-03
**Status:** Historical — informed the Communities v2.0 redesign, which has
since been implemented. The flat-SMPL-governance model (no coordinator) is
now the production architecture. See
`.claude/docs/rekindle-communities-architecture.md` for the current spec
and the top-level `docs/protocol.md` / `docs/architecture.md` for how it
landed in code.

---

## The Problem

Our rotating coordinator model has 20 identified bugs including 4 critical system-breaking
issues. But the bugs are symptoms of deeper architectural problems:

1. **No production P2P chat system uses dynamic leader election for group coordination**
2. **Veilid DFLT records have a single owner keypair** — new coordinators can't write to the manifest
3. **DHT is eventually consistent** — using it for coordinator election creates split-brain
4. **Fire-and-forget over ephemeral routes** guarantees message loss without ACK/retry
5. **The heartbeat system is dead code** — shutdown channels are immediately dropped

These aren't fixable with patches. The architecture needs rethinking.

---

## What Production Systems Actually Do

### The Universal Pattern: Static Creator-as-Authority

Every production P2P group chat uses the same fundamental pattern:

| System | Coordinator Model | Key Insight |
|--------|------------------|-------------|
| **Session v2** | Admin with shared seed keypair | Admins promoted, but original seed holder is ultimate authority |
| **Quiet** | Owner as Certificate Authority (Tor) | Owner must be online for joins; existing members chat via CRDT |
| **Briar** | Creator only; group dissolves on leave | No succession mechanism at all |
| **Nostr NIP-29** | Relay IS the coordinator | Groups migrate between relays manually |
| **Berty/Wesh** | No coordinator — OrbitDB CRDTs | Lamport clocks for ordering, O(N) key exchange |
| **Matrix P2P** | No coordinator — State Resolution v2 | Deterministic conflict resolution algorithm |
| **GossipSub** | No coordinator — mesh pubsub | Self-healing mesh of D peers per topic |

**No one does rotating election.** The two approaches that work are:
1. **Static authority with admin delegation** (Session, Quiet, Briar, Nostr)
2. **No coordinator at all — CRDTs/mesh** (Berty, Matrix, GossipSub)

### Why Rotating Election Fails in P2P

- DHT is eventually consistent — two nodes can compute different election winners simultaneously
- No fencing tokens or leases exist in Veilid's DHT
- Private routes are ephemeral (~5 min TTL) — elected coordinator's route goes stale
- No atomic state handover between old and new coordinator
- The manifest (DFLT record) can only be written by the original owner keypair

---

## Veilid-Specific Constraints

### What VeilidChat Does (1:1 only, no group chat exists)

VeilidChat uses **dual DHTLog + local reconciliation**:
- Each peer owns an append-only DHTLog (ring-buffer over DHT subkeys)
- Both peers watch each other's logs
- Messages merged locally by timestamp + deterministic author tiebreaking
- No coordinator needed for 1:1

### Veilid DHT Capabilities

| Feature | Details | Implication |
|---------|---------|-------------|
| DFLT schema | Single-owner, up to 65535 subkeys | Good for manifest, but only owner can write |
| SMPL schema | Multi-writer, max 256 members, 1024 subkeys | Good for member registry/presence, immutable after creation |
| `watch_dht_values` | Unreliable — no fallback when watch can't be established (Issue #377) | Cannot rely on for real-time; only for slow state changes |
| `app_message` | Fire-and-forget, 32KB max, anonymous via private route | Correct for group relay |
| `app_call` | Request-response, blocks caller | Wrong for fan-out (would serialize N responses) |
| Private routes | Expire ~5min, no self-loopback | Must refresh regularly; coordinator can't send to self |
| `set_dht_value` return | Returns `Some(data)` if network has newer value | Built-in optimistic concurrency check |

### Key Veilid Limitation

**SMPL schemas are immutable after creation.** You cannot add members to an existing SMPL
record — you must create a new record with the updated member list. This is why scaling
beyond 256 members requires record chaining.

### No Other Veilid Group Chat Exists

Rekindle is the first Veilid application attempting group chat. VeilidChat is 1:1 only.
Other projects (veilid_duplex, DDCP) are peer-to-peer, not group-aware.

---

## Code Audit: Critical Bugs (Ranked)

### CRITICAL (System-Breaking)

| # | Bug | Location | Impact |
|---|-----|----------|--------|
| 1 | Heartbeat shutdown_tx immediately dropped | election.rs:117,146 | Heartbeat/monitor never run. Re-election is dead code. |
| 2 | `member_registry_key` never set after join | community_service.rs:279 | Joiners can never become coordinator. Election reads empty list. |
| 3 | `is_community_owner` only checks local user | relay.rs:1567-1578 | Owner permissions wrong for all remote members |
| 4 | Fan-out fire-and-forget, no retry | relay.rs:1052-1068 | Silent message loss when routes go stale |

### MAJOR (Significant Breakage)

| # | Bug | Location | Impact |
|---|-----|----------|--------|
| 5 | Split-brain election (no consensus) | election.rs | Two coordinators simultaneously, divergent state |
| 6 | Coordinator state lost on re-election | relay.rs `RelayService::new()` | Empty online_members, lost automod/raid state |
| 7 | Channel overwrites always empty | relay.rs:1252-1267 | Channel-specific permissions don't work |
| 8 | Concurrent joins lost-update race | relay.rs:468-593 | Simultaneous joins overwrite each other in registry |
| 9 | Coordinator can't see own broadcasts | veilid_service.rs:150 | System messages, role changes invisible to coordinator |
| 10 | Manifest requires original owner keypair | manifest.rs | **Fundamentally breaks rotating coordinator model** |

### MODERATE

| # | Bug | Location | Impact |
|---|-----|----------|--------|
| 11 | Stale online_members never evicted | relay.rs:40 | Wasted delivery attempts to dead routes |
| 12 | refresh_online_members O(N) sequential | relay.rs:126-153 | 50 members = 30s blocking |
| 13 | Ban list not checked on join | relay.rs:468-593 | Banned users can rejoin |
| 14 | Heartbeat writes empty route blob | heartbeat.rs:90 | All coordinator communication breaks |
| 15 | Invite TOCTOU race | relay.rs:1664+ | Single-use invites can be used multiple times |

---

## Recommended Approaches

### Option A: Static Owner-as-Coordinator (Session/Quiet Model)

**How it works:**
- Community creator owns the manifest keypair permanently
- Creator is always the coordinator (relay) when online
- Creator can promote admins who get delegated authority
- When creator is offline: members can still chat if another member relays (admin fallback)
- Join requires creator (or admin with delegation) to be online

**Pros:**
- Matches every production P2P system
- No election complexity, no split-brain
- Manifest keypair ownership is natural (creator owns the DFLT record)
- Simple mental model for users

**Cons:**
- Creator must be online for joins and moderation
- Single point of failure (partially mitigated by admin delegation)
- Doesn't match the "Discord server anyone can run" vision

**Effort:** Moderate — remove election/heartbeat, simplify relay to always-owner model

### Option B: Admin Pool with Deterministic Fallback (Hybrid)

**How it works:**
- Creator owns manifest, but shares a **delegation secret** with promoted admins
- Any admin can act as coordinator when online
- Deterministic priority: creator > admin by join order > oldest member
- No election — just "who's online and has highest priority?"
- Admin writes to manifest via the creator's delegated keypair (shared secret)

**Pros:**
- Multiple potential coordinators without election
- No split-brain (deterministic priority, no voting)
- Manifest write access via shared delegation secret
- Better availability than single-owner

**Cons:**
- Shared secrets for manifest writing (security tradeoff)
- Still requires at least one admin online
- More complex key management

**Effort:** Moderate-high — need delegation key scheme, admin priority logic

### Option C: CRDT-Based No-Coordinator (Berty/Matrix Model)

**How it works:**
- No coordinator at all
- Each member has their own DHTLog (append-only, like VeilidChat)
- Members watch each other's logs and merge locally
- State resolution algorithm for conflicts (Matrix-style)
- Permissions enforced locally by each member (reject unauthorized writes)

**Pros:**
- No single point of failure
- No election, no relay bottleneck
- Scales naturally
- Matches VeilidChat's architecture

**Cons:**
- O(N) DHT watches per member (N members watching N logs)
- Complex conflict resolution needed
- Moderation is harder (kick/ban enforced by each member ignoring the banned user)
- DHT watches are unreliable (Veilid Issue #377)
- Complete rewrite of community system

**Effort:** Very high — fundamental redesign of message flow

### Option D: Fix Current Architecture (Pragmatic)

**How it works:**
- Keep coordinator-relay model but make it actually work
- Fix the 15 critical/major bugs
- Accept the rotating coordinator won't work — make creator the permanent coordinator
- Keep the relay infrastructure, just remove the election complexity

**This is effectively Option A implemented as a refactor of existing code.**

**Effort:** Low-moderate — fix bugs, remove election code, simplify

---

## Recommendation

**Option D (pragmatic fix → Option A)** is the clear winner for shipping something that works:

1. **Remove election/heartbeat entirely** — creator is always coordinator
2. **Fix the 4 critical bugs** (heartbeat already removed, member_registry_key, is_community_owner, fan-out retry)
3. **Fix the coordinator loopback** — coordinator sees all its own broadcasts
4. **Add message delivery ACK** — members confirm receipt, coordinator retries on timeout
5. **Add route refresh** — periodic presence updates refresh stale routes in online_members
6. **Fix concurrent join race** — serialize join processing with a tokio Mutex

Later evolution path: Option A → Option B (admin delegation) when the basic system is stable.

---

## Sources

- [Session Protocol Technical Info](https://getsession.org/session-protocol-technical-information)
- [Session Groups v2 — DeepWiki](https://deepwiki.com/oxen-io/libsession-util/5.1-group-keys-and-encryption)
- [Quiet FAQ / Architecture](https://github.com/TryQuiet/quiet/wiki/Quiet-FAQ)
- [Briar — How it works](https://briarproject.org/how-it-works/)
- [Matrix State Resolution v2](https://matrix.org/docs/older/stateres-v2/)
- [Nostr NIP-29](https://github.com/nostr-protocol/nips/blob/master/29.md)
- [Berty Wesh Protocol](https://berty.tech/docs/protocol/)
- [GossipSub v1.0 Spec](https://github.com/libp2p/specs/blob/master/pubsub/gossipsub/gossipsub-v1.0.md)
- [p2panda Access Control](https://p2panda.org/2025/07/28/access-control.html)
- [Veilid Developer Book](https://veilid.gitlab.io/developer-book/)
- [Veilid RoutingContext Docs](https://docs.rs/veilid-core/latest/veilid_core/struct.RoutingContext.html)
- [Veilid GitLab Issue #377 — Watch Reliability](https://gitlab.com/veilid/veilid/-/issues/377)
- [VeilidChat Source](https://gitlab.com/veilid/veilidchat)
- [veilid_duplex](https://github.com/stillonearth/veilid_duplex)
