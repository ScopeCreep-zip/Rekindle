# Coordinator Architecture Audit

## Date: 2026-03-03

## Critical Findings (System-Breaking)

### 1. Heartbeat/Monitor Shutdown Channels Immediately Dropped
**Location:** `election.rs:117, 146`

The `_hb_shutdown_tx` sender is created inside an `if` block and immediately dropped when the block exits. Since the only sender for `hb_shutdown_rx` is dropped, `shutdown_rx.recv()` returns `None` immediately, causing the heartbeat to exit on its very first select iteration.

**Impact:** The coordinator heartbeat and member heartbeat monitor never actually run. This means:
- Coordinator stops writing heartbeats after initial election
- Members never detect coordinator failure
- Re-election never triggers — the entire re-election mechanism is dead code

### 2. `is_community_owner` Only Checks Local User
**Location:** `relay.rs:1567-1578`

The function checks if `pseudonym == my_pseudonym_key` then checks `my_role_ids`. It can only return `true` for the local user. When the coordinator checks permissions for a remote member who IS the owner, this always returns `false`.

### 3. `member_registry_key` Never Populated After Join
**Location:** `community_service.rs:279`, `veilid_service.rs handle_join_accepted`

The joiner sets `member_registry_key: None` and `handle_join_accepted` never updates it. This means:
- Joiner's election service cannot read the member index
- If this member becomes coordinator, `refresh_online_members` bails (registry_key is None)
- All fan-out from this coordinator-turned-member fails silently

### 4. No Message Delivery Confirmation or Retry for Fan-Out
**Location:** `relay.rs:1052-1068`

Fan-out is completely fire-and-forget with debug-level logging. When Veilid routes go stale, messages are silently lost. No retry, no ACK, no queue, no way for members to request missed messages.

## Major Problems

### 5. Split-Brain Election (No Consensus)
Deterministic hash assumes all nodes see the same member list simultaneously. DHT eventual consistency means two nodes can compute different winners and both write themselves as coordinator.

### 6. Coordinator State Lost on Re-election
`RelayService::new()` creates fresh instance with empty online_members, default automod, default raid state. No handover protocol.

### 7. Channel Permission Overwrites Always Empty
`get_channel_overwrites` is a stub returning `Vec::new()`. Channel-specific restrictions don't work.

### 8. Concurrent Joins Cause Lost Updates
Join handling in `tokio::spawn` without serialization. Two simultaneous joins → read-modify-write race on member index → one member silently lost.

### 9. Coordinator Doesn't See Its Own Broadcasts
Loopback skips `handle_relayed_envelope` for self (`is_from_self` check). The `emit_local_member_joined` is a one-off patch covering only join events. System messages, role changes, kicks, etc. are all invisible to the coordinator's own frontend.

### 10. Manifest DFLT Record Requires Original Owner Keypair
New coordinators cannot write to the manifest because DFLT records have a single owner keypair. Only the original creator can update channels, roles, bans, invites, coordinator info. **This fundamentally breaks the rotating coordinator model.**

## Moderate Problems

### 11. Stale `online_members` Never Evicted
No TTL or periodic cleanup. Crashed members' stale routes remain forever.

### 12. `refresh_online_members` is O(N) Sequential DHT Reads
Each member requires a separate DHT read (500ms-2s each). 50 members = 30+ seconds blocking.

### 13. Ban List Not Checked on Join
Join handler validates invites and raid protection but never checks the ban list.

### 14. Heartbeat Can Write Empty Route Blob
`unwrap_or_default()` on missing route blob writes empty vec to DHT, breaking all coordinator communication.

### 15. Invite Validation TOCTOU Race
Read-modify-write on invite use_count without any locking.

## Fundamental Architecture Questions

The audit reveals that the "rotating coordinator" model has several design assumptions that don't hold:

1. **DHT is not a coordination primitive** — It's eventually consistent, not strongly consistent. Using it for coordinator election is fundamentally flawed.
2. **Private routes are ephemeral** — Veilid routes expire and go stale. Building a relay on top of them without route refresh/confirmation is fragile.
3. **Fire-and-forget in P2P is message loss** — Without ACKs and retransmission, any network hiccup means lost messages.
4. **Single-owner DHT records can't have rotating writers** — The manifest architecture contradicts the coordinator model.
