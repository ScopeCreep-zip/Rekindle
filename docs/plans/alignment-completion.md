# Alignment Completion Plan

Closes every outstanding item from `docs/research/alignment-audit.md`
plus the findings accumulated during the deduplication branch.

Every signature below was read out of the tree before being written
here. Where a plan step says "call X", X exists and its exact shape is
quoted. Where something has to be built, it says so.

**On the code below.** Every signature was read out of the tree, and
that check caught three of my own errors before they reached this file:
`services/community/gossip.rs::send_to_channel_peers` is voice-session
bound and unusable for a directed reply; there is no
`state_helpers::dht_manager` (the pattern is `DHTManager::new(rc)`); and
`IpcResponse::error` takes a code as well as a message. Treat anything
here as verified against the tree at the commit this plan landed on, and
re-check before assuming it still holds.

**Ordering rationale:** phases 1–3 are independent, small, and each has
a named failure mode you can reproduce. Phase 4 is the structural one
and depends on nothing in 1–3, but is far larger — it is last so the
cheap correctness wins land first.

---

## Phase 1 — Sync response: bound it, and answer the asker

**Defect** (audit §2.1). `handle_sync_request` packs up to `LIMIT 500`
messages into a `SyncResponse` and broadcasts it over the gossip mesh.
`SyncedMessage.sender_key` is a 64-char hex pseudonym, so 500 rows is
32,000 bytes of sender keys alone — before bodies. veilid-core caps
`app_message` at 32,768 and **errors** rather than truncating:

```rust
// veilid-core-0.5.7 rpc_processor/coders/operations/operation_app_message.rs
const MAX_APP_MESSAGE_MESSAGE_LEN: usize = 32768;
if message.len() > MAX_APP_MESSAGE_MESSAGE_LEN {
    return Err(RPCError::internal("AppMessage message too long to set"));
}
```

The result is dropped with `let _ =`, so the failure is silent.

### 1.1 Thread the requester through

`src-tauri/src/services/veilid/control_moderation.rs:19` already has
`sender_pseudonym: &str` and passes it to neighbouring handlers (lines
33, 47). It does not pass it here. Change the call:

```rust
// control_moderation.rs — ControlPayload::SyncRequest arm
ControlPayload::SyncRequest { channel_id, since_timestamp } => {
    handle_sync_request(
        app_handle,
        state,
        community_id,
        sender_pseudonym,          // NEW
        &channel_id,
        since_timestamp,
    );
}
```

and the signature in `services/veilid/control_sync.rs`:

```rust
pub(crate) fn handle_sync_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    requester_pseudonym: &str,     // NEW
    channel_id: &str,
    since_timestamp: u64,
)
```

### 1.2 Bound by bytes, not rows

Keep `LIMIT 500` as a cheap first cut, then fill a response until the
encoded size approaches the cap. Add to `control_sync.rs`:

```rust
/// Ceiling for one `SyncResponse` payload.
///
/// veilid-core rejects an `app_message` over
/// `MAX_APP_MESSAGE_MESSAGE_LEN` (32 768) with
/// `RPCError::internal("AppMessage message too long to set")` — it does
/// not truncate, so an oversize response is not a degraded reply, it is
/// no reply. The margin covers the `CommunityEnvelope` wrapper, the
/// signature, and Cap'n Proto framing around the message list.
const SYNC_RESPONSE_BUDGET_BYTES: usize = 28 * 1024;

/// Take messages oldest-first until the next one would not fit.
///
/// Oldest-first because the requester is filling a gap backwards from
/// `since_timestamp`; a truncated newest-first page would leave a hole
/// in the middle that no later request asks for again.
fn fit_within_budget(mut msgs: Vec<SyncedMessage>) -> Vec<SyncedMessage> {
    let mut out = Vec::with_capacity(msgs.len());
    let mut used = 0usize;
    for m in msgs.drain(..) {
        // 64 hex chars of sender_key + body + the three integers, plus
        // per-field JSON overhead. Measured, not guessed: serialising
        // the row is cheaper than being wrong about its size.
        let size = serde_json::to_vec(&m).map(|v| v.len()).unwrap_or(usize::MAX);
        if used + size > SYNC_RESPONSE_BUDGET_BYTES {
            break;
        }
        used += size;
        out.push(m);
    }
    out
}
```

Apply between the query and the envelope build, and log when it bites —
a truncated page means the requester needs another round:

```rust
let total = messages.len();
let messages = fit_within_budget(messages);
if messages.len() < total {
    tracing::debug!(
        community = %community_id_owned,
        sent = messages.len(),
        withheld = total - messages.len(),
        "sync response truncated to fit the app_message ceiling"
    );
}
```

### 1.3 Send it to the requester, not the mesh

`services/community/gossip.rs::send_to_channel_peers` is **not** usable:
it is voice-session bound and errors when no voice session is active on
that channel. The crate-level one is the right primitive:

```rust
// crates/rekindle-gossip/src/mesh_broadcast.rs
pub async fn send_to_channel_peers<D: GossipDeps>(
    deps: Arc<D>,
    community_id: &str,
    envelope: &CommunityEnvelope,
    peers: Vec<PeerInfo>,
) -> Result<(), GossipError>
```

It already forces `ttl = 0`, which is exactly right here — a directed
history reply must never be gossip-forwarded.

Add a directed wrapper beside the existing ones in
`services/community/gossip.rs`:

```rust
/// Send one envelope to exactly one member.
///
/// Distinct from `send_to_channel_peers` above, which is bound to the
/// active voice session — this one takes the peer explicitly and is for
/// directed replies (sync history, and anything else answering a
/// specific asker rather than telling the community).
pub async fn send_to_member(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    peer_pseudonym: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), String> {
    let route_blob =
        crate::services::community::routes::resolve_member_route(
            state, pool, community_id, peer_pseudonym,
        )
        .await
        .ok_or_else(|| format!("no route for {peer_pseudonym}"))?;
    let adapter = build_adapter(state).ok_or("app handle / db pool unavailable")?;
    rekindle_gossip::send_to_channel_peers(
        Arc::new(adapter),
        community_id,
        envelope,
        vec![rekindle_gossip::PeerInfo {
            pseudonym_key: peer_pseudonym.to_string(),
            route_blob,
        }],
    )
    .await
    .map_err(|e| e.to_string())
}
```

`resolve_member_route` exists at `services/community/routes.rs:13`:

```rust
pub async fn resolve_member_route(
    state: &Arc<AppState>, pool: &DbPool,
    community_id: &str, peer_pseudonym: &str,
) -> Option<Vec<u8>>
```

Then in `handle_sync_request`, replace

```rust
let _ = crate::services::community::send_to_mesh(&state, &community_id_owned, &envelope);
```

with

```rust
if let Err(e) = crate::services::community::gossip::send_to_member(
    &state, &pool, &community_id_owned, &requester_owned, &envelope,
).await {
    tracing::warn!(
        community = %community_id_owned,
        requester = %requester_owned,
        error = %e,
        "sync response delivery failed"
    );
}
```

The `let _ =` goes: a failed history reply is worth a line.

### 1.4 Test

`src-tauri/src/services/veilid/control_sync.rs` tests module:

- `fit_within_budget_stops_before_the_ceiling` — build 500 rows with
  64-char sender keys and 200-byte bodies, assert the serialised result
  is `<= SYNC_RESPONSE_BUDGET_BYTES` and that fewer than 500 came back.
- `fit_within_budget_keeps_everything_when_it_fits` — 5 rows, all 5 back.
- `fit_within_budget_is_oldest_first` — assert the retained slice is a
  prefix of the input.

---

## Phase 2 — Release relay routes on revoke

**Defect** (audit §2.3). `services/relay/offer.rs` allocates a private
route per volunteered friend and stores its id in
`strand_relay_volunteered`. `revoke_relay` deletes the row and sends
`RelayWithdraw` without releasing the route.

Veilid's release is local and cheap:

```rust
// veilid-core-0.5.7 veilid_api/api.rs
/// Local, no network round-trip. Releasing a route id that is unknown,
/// already released, or malformed ... returns InvalidArgument
pub fn release_private_route(&self, route_id: RouteId) -> VeilidAPIResult<()>
```

Read the id **before** the delete, in the same `db_call`:

```rust
// services/relay/offer.rs — revoke_relay
let pseudonym_q = pseudonym.clone();
let friend_q = friend.clone();
let route_id: Option<String> = db_call(pool, move |conn| {
    let existing: Option<String> = conn
        .query_row(
            "SELECT relay_route_id FROM strand_relay_volunteered \
             WHERE owner_key = ?1 AND friend_public_key = ?2",
            rusqlite::params![pseudonym_q, friend_q],
            |row| row.get(0),
        )
        .optional()?;
    conn.execute(
        "DELETE FROM strand_relay_volunteered WHERE owner_key = ?1 AND friend_public_key = ?2",
        rusqlite::params![pseudonym_q, friend_q],
    )?;
    Ok(existing)
})
.await?;

// The route outlives the row unless we say otherwise. Releasing is
// local and idempotent-ish: an unknown or already-released id returns
// InvalidArgument, which is not worth failing the revoke over.
if let Some(id) = route_id {
    if let (Some(api), Ok(parsed)) = (
        state_helpers::veilid_api(state),
        id.parse::<veilid_core::RouteId>(),
    ) {
        if let Err(e) = api.release_private_route(parsed) {
            tracing::debug!(route_id = %id, error = %e, "relay route release");
        }
    }
}
```

Note the single `db_call` — a SELECT-then-DELETE in two calls can race
a concurrent revoke and leak the id it was about to read.

### 2.1 Test

`services/relay/offer.rs` tests: `revoke_reads_route_id_before_delete` —
insert a row, revoke, assert the row is gone **and** the returned id
matched what was inserted. (The release itself needs a live Veilid API
and is covered by the integration path, not a unit test.)

---

## Phase 3 — Stop watching a removed or blocked friend

**Defect** (audit §2.2). Friend profile records are opened
(`chat_runtime.rs`, `message_service/transport.rs` ×2,
`presence_adapter/friend_deps.rs`) and watched
(`friend_deps.rs:141 watch_friend_subkeys`), but registered in neither
inventory. `friend_runtime/remove.rs` and `block.rs` contain no watch,
close, or cancel call.

Veilid ties the two together:

> `close_dht_record` — The release half of the open/close pair;
> **cancels the record's watch** (in the background) and drops any
> associated transaction.

So closing the record is the cancel.

### 3.1 Track the friend record when it is opened

`watch_friend_subkeys` is the single place a friend record becomes
watched, so it is the right place to register it:

```rust
// presence_adapter/friend_deps.rs, in watch_friend_subkeys,
// after the successful watch_dht_values call
let active = rc.watch_dht_values(record_key, Some(subkey_range), None, None)
    .await
    .map_err(|e| PresenceError::Dht(e.to_string()))?;
if active {
    // A watched record must be closable, and closing is what cancels
    // the watch. Without this the watch outlives the friendship.
    crate::state_helpers::track_open_records(
        &self.state,
        std::slice::from_ref(&dht_record_key.to_string()),
    );
}
Ok(active)
```

`track_open_records(&Arc<AppState>, &[String])` exists at
`state_helpers/dht_records.rs:56` and feeds the same global set
`cleanup.rs::close_tracked_records` drains at logout and app exit — so
this also fixes the process-lifetime accumulation, not only the
per-friend case.

### 3.2 Close on remove and block

New helper next to the other DHT record helpers:

```rust
// state_helpers/dht_records.rs
/// Close one record and drop it from the tracking set.
///
/// Closing is what cancels the record's watch (veilid-core:
/// "the release half of the open/close pair; cancels the record's
/// watch"), so this is the only way to stop observing a peer.
pub async fn close_and_untrack(state: &Arc<AppState>, key: &str) {
    let rc = { state.node.read().as_ref().map(|nh| nh.routing_context.clone()) };
    if let (Some(rc), Ok(parsed)) = (rc, key.parse::<veilid_core::RecordKey>()) {
        if let Err(e) = rc.close_dht_record(parsed).await {
            tracing::debug!(key, error = %e, "close friend record");
        }
    }
    untrack_records(state, std::slice::from_ref(&key.to_string()));
}
```

### 3.2b The call sites — the key is already in scope

The plan originally said to read `profile_dht_key` back from the
friends table before the row is deleted. Reading the code says
otherwise: `remove.rs` already holds it as `dht_key` at the point it
tears down routing, and already calls
`mgr.unregister_friend_dht_key(dht_key)` there.

That existing call is **not** a close. `unregister_friend_dht_key`
(`state/runtime.rs:172`) only drops an entry from `dht_key_to_friend`,
the map that answers "which friend does this value-change belong to".
The record stays open and the watch stays live.

`services/friend_runtime/remove.rs`, in the existing block:

```rust
{
    let mut dht_mgr = state_clone.dht_manager.write();
    if let Some(mgr) = dht_mgr.as_mut() {
        if let Some(ref dht_key) = dht_key {
            mgr.unregister_friend_dht_key(dht_key);
        }
        mgr.manager.invalidate_route_for_peer(&pk_clone);
    }
}
// NEW — unregistering the mapping does not stop the watch. Closing the
// record is what cancels it (veilid-core: close_dht_record is "the
// release half of the open/close pair; cancels the record's watch").
// Outside the dht_manager guard: close_and_untrack takes the same lock.
if let Some(ref dht_key) = dht_key {
    state_helpers::close_and_untrack(&state_clone, dht_key).await;
}
```

Note the placement: `close_and_untrack` acquires `dht_manager.write()`
itself, so it must run **after** the guard above is dropped — the
`parking_lot` guards here are not reentrant.

`services/friend_runtime/block.rs` gets the same addition beside its own
`invalidate_route_for_peer(&public_key)` at line 68. Blocking that
leaves a live watch is the worse of the two: we keep observing someone
the user explicitly cut off.

### 3.3 Test

`friend_runtime/remove.rs` tests: `remove_untracks_the_profile_record` —
seed a friend with a `profile_dht_key`, track it, remove, assert the key
is no longer in the tracking set. (The Veilid close needs a live node;
the tracking half is what regresses silently and is what the test
pins.)

---

## Phase 4 — Make the daemon the substrate

**Defect** (audit §1 and §3, one problem seen from two ends). 258 Tauri
commands, 64 daemon IPC variants, 211 with no daemon counterpart. And
`src-tauri` never references `SubscriptionEvent` — the Tauri frontend
gets `channels::presence_channel::PresenceEvent`, the CLI gets
`rekindle_types::subscription_events::PresenceEvent`.

This is a phase, not a patch, and it does not need to land at once. The
order below makes each step independently shippable.

### 4.1 One event vocabulary (do this first)

The event split is what makes the command gap self-perpetuating: every
new desktop feature invents a Tauri-only event, so the daemon never
needs the command. Fix the vocabulary and the surface follows.

`rekindle_types::subscription_events::SubscriptionEvent` is already the
richer, round-trippable one:

```rust
pub enum SubscriptionEvent {
    ChannelMessage(ChannelMessageEvent),
    Typing(TypingEvent),
    Presence(PresenceEvent),
    Membership(MembershipEvent),
    Friend(FriendEvent),
    Crypto(CryptoEvent),
    Voice(VoiceEvent),
    Governance(GovernanceEvent),
    ...
}
```

**Do not rewrite the ~150 emit sites.** `event_dispatch.rs` already
funnels every emit through one queue, and the envelope is already
type-erased:

```rust
// src-tauri/src/event_dispatch.rs
struct EmitEnvelope {
    channel: String,
    payload: serde_json::Value,
}

pub fn spawn_dispatch_loop(app: AppHandle, dispatch: &Arc<EventDispatch>) {
    // ...
    while let Some(envelope) = rx.recv().await {
        let _ = app.emit(&envelope.channel, &envelope.payload);
    }
}
```

Because the payload is already `serde_json::Value`, no `From` impl on
the channel types is needed — an emitter that produces a
`SubscriptionEvent` can push through the same queue unchanged. Add one
helper beside `emit_live` / `emit_journaled`:

```rust
/// Emit a daemon-vocabulary event to the webview.
///
/// The CLI receives `SubscriptionEvent` over IPC; this puts the *same*
/// value on the Tauri channel, so both frontends observe one event for
/// one action. `emit_live` and `emit_journaled` keep working untouched
/// — this is an addition, not a migration, and emitters move onto it
/// family by family.
pub fn emit_subscription(app: &AppHandle, event: &SubscriptionEvent) {
    emit_live(app, channel_for(event), event);
}

/// Which Tauri channel a daemon event belongs on.
///
/// The channel names are the existing contract the frontend already
/// listens on (`"presence-event"`, `"voice-event"`, …), so the
/// frontend's `listen()` calls do not move — only the payload shape
/// converges.
fn channel_for(event: &SubscriptionEvent) -> &'static str {
    match event {
        SubscriptionEvent::Presence(_) => "presence-event",
        SubscriptionEvent::Voice(_) => "voice-event",
        SubscriptionEvent::ChannelMessage(_) | SubscriptionEvent::Typing(_) => "chat-event",
        SubscriptionEvent::Membership(_) | SubscriptionEvent::Governance(_) => "community-event",
        SubscriptionEvent::Friend(_) => "friend-event",
        SubscriptionEvent::Crypto(_) => "community-event",
    }
}
```

Then migrate one family at a time. Presence first, because it is the
smallest and both sides already have the type:

1. Replace `emit_live(app, "presence-event", &channels::PresenceEvent::FriendOnline { .. })`
   with `emit_subscription(app, &SubscriptionEvent::Presence(PresenceEvent::FriendChanged { .. }))`.
2. Update the frontend's `presence-event` handler to the new payload
   shape (`src/ipc/channels/presence_events.ts`).
3. Delete the now-unused `channels::presence_channel::PresenceEvent`
   variant, and remove its `DUPLICATE_TYPE_EXCEPTIONS` entry.

Step 3 is the ledger: the exception list shrinks by one per family, and
when `PresenceEvent`, `VoiceEvent` and `CommunityDetail` are all gone,
the vocabularies have converged. Order: presence → voice → membership →
governance → chat → friend.

### 4.2 Close the command gap by family

For each family, the daemon gets the `IpcRequest` variants the desktop
already has. The pattern is fixed — three edits per command:

```rust
// 1. crates/rekindle-node/src/ipc/protocol/mod.rs — the variant
ThreadCreate { community: String, channel: String, title: String, body: String },

// 2. crates/rekindle-node/src/daemon/dispatch/router.rs — the arm
IpcRequest::ThreadCreate { community, channel, title, body } =>
    channel::handle_thread_create(ctx, state, &community, &channel, &title, &body).await,

// 3. the handler, delegating to the same crate the Tauri command uses
```

Every one of the 211 is listed below. The earlier draft of this plan
had six waves with estimated counts (`~25`, `~20`, …) that summed to
~122 — barely half the gap — and the families it named left device
pairing, relay, game servers, notifications, channel admin, friends
admin and voice moderation entirely unaccounted for. Estimates are how a
plan ends up unexecutable, so this is the enumeration instead.

Generated from `invoke.rs` against `IpcRequest`, matched on token sets
so `join_community` ↔ `CommunityJoin` counts as covered.


**A threads/pins/reactions — 15**

```
add_reaction
archive_thread
create_thread
get_active_threads
get_channel_pins
get_channel_threads
get_thread_messages
pin_attachment
pin_message
remove_reaction
send_call_reaction
send_thread_message
send_typing
unarchive_thread
unpin_message
```


**B events/polls — 13**

```
cancel_event
close_poll
create_event
create_poll
delete_event
edit_event
event_resume
get_events
get_poll_results
list_event_attendees
rsvp_event
set_event_rsvp
vote_poll
```


**C dm+calls — 16**

```
accept_dm_call
accept_dm_invite
accept_group_call
decline_dm_call
decline_dm_invite
decline_group_call
end_dm_call
end_group_call
get_dm_messages
get_missed_calls
list_dms
mute_caller_temp
send_call_media_state
start_dm
start_dm_call
start_group_call
```


**D governance admin — 24**

```
create_category
delete_automod_rule
delete_category
edit_role
expand_community_segment
get_community_policy
get_onboarding_config
get_roles
get_welcome_screen
list_automod_rules
mark_onboarding_complete
remove_community_member
rename_category
reorder_categories
server_deafen_member
server_mute_member
set_automod_rule
set_channel_forum_tags
set_channel_overwrite
set_community_policy
set_onboarding_config
set_slowmode
set_welcome_screen
submit_onboarding_answers
```


**E attachments/expressions — 8**

```
delete_emoji
download_attachment
list_expressions
play_soundboard
upload_attachment
upload_emoji
upload_soundboard_sound
upload_sticker
```


**F read-only — 13**

```
audit_export
audit_verify
debug_gossip_state
get_audit_log
get_channel_messages
get_communities
get_community_analytics
get_community_details
get_message_history
get_older_channel_messages
get_unread_counts
search_messages
vault_diagnostics
```


**G channel admin — 9**

```
edit_channel_message
forward_channel_message
mark_channel_read
move_channel
rename_channel
reorder_channels
set_active_channel
set_channel_notification_level
set_channel_topic
```


**H friends/social — 20**

```
accept_request
accept_session_reset
block_user
cancel_invite
cancel_request
create_friend_group
decline_session_reset
friendship_scan_now
generate_invite
get_blocked_users
get_friends
get_outgoing_invites
get_pending_requests
list_volunteered_relay_friends
move_friend_to_group
reject_request
rename_friend_group
request_to_speak
respond_to_speak_request
unblock_user
```


**I device sync/pairing — 12**

```
accept_pairing_code
ensure_personal_sync_record
read_paired_devices
read_sync_manifest
read_sync_preferences
read_sync_read_state
run_background_sync
start_pairing_session
write_paired_devices
write_sync_manifest
write_sync_preferences
write_sync_read_state
```


**J relay/push — 6**

```
list_push_relay_registrations
list_received_relay_offers
register_with_push_relay
revoke_relay
unregister_with_push_relay
volunteer_relay
```


**K game presence — 5**

```
add_game_server
get_game_name
get_game_servers
launch_game_to_server
remove_game_server
```


**L notifications/prefs — 11**

```
get_community_default_notification_level
get_do_not_disturb
get_notification_sound
get_preferences
get_quiet_hours
set_community_default_notification_level
set_do_not_disturb
set_nickname
set_notification_sound
set_preferences
set_quiet_hours
```


**M voice moderation — 5**

```
get_stage_hand_raises
send_voice_message
set_deafen
set_mute
set_voice_mode
```


**N identity/lifecycle — 8**

```
check_for_updates
delete_identity
dev_disable_watch
lifecycle_current
login
logout
pqxdh_bundle_info
update_community_profile
```


### Deliberately not ported — 41

Window management, native capture, media plumbing and image rendering
are GUI concepts. A CLI has no window to open and no canvas to paint,
so these stay desktop-only. Naming them is the point: an unlisted gap
reads as an oversight.

```
default_media_capabilities
derive_video_stream_id
dm_peer_video_decode_codecs
fetch_link_preview
force_native_keyframes
generate_pairing_qr_svg
get_avatar
get_community_avatar_data_url
get_link_previews_enabled
list_audio_devices
list_native_video_devices
native_video_active
native_video_capture_available
notify_video_topology_change
open_call_window
open_chat_window
open_community_window
open_dm_window
open_profile_window
open_settings_window
register_community_video_channel
register_dm_video_channel
register_native_preview_channel
report_local_video_capabilities
report_media_capture_error
send_video_bandwidth_estimate
send_video_frame
send_video_frame_ack
send_video_keyframe_request
set_audio_devices
set_avatar
set_community_avatar
set_community_banner
set_link_previews_enabled
show_buddy_list
show_os_notification
start_native_video
stop_native_video
unregister_community_video_channel
unregister_dm_video_channel
unregister_native_preview_channel
```

### Needs a decision — 5

```
list_identities
mark_read
prepare_chat_session
reset_signal_session
send_message
```

`send_message` / `mark_read` / `prepare_chat_session` are the 1:1 DM
path, which the daemon covers under different names (`DmSend`,
`DmInbox`) — confirm the mapping rather than porting duplicates.
`list_identities` and `reset_signal_session` are genuine gaps.

**Total: 165 ported across 14 waves, 41 deliberately desktop-only,
5 to decide = 211.**

## Phase 5 — The smaller outstanding findings

Each is small and independent; none blocks the others.

### 5.1 The daemon stores a policy it never reads

`admin.rs:179` writes `*ctx.policy.write() = policy.clone()` and nothing
reads it back. `min_hop_count`, `require_signature_verification` and
`max_gossip_ttl` are enforced by the CLI against its own config
(`config/loader.rs::enforce_policy`) and by nothing daemon-side, so a
policy pushed to a running daemon changes no behaviour.

`dispatch()` at `daemon/dispatch/router.rs:15` is the one place every
request passes through, which makes it the guard:

```rust
pub async fn dispatch(ctx: &DaemonContext, request: IpcRequest) -> IpcResponse {
    // Admin policy is a ceiling on what a request may ask for, so it is
    // checked before the request is routed — not inside each handler,
    // where a new handler would silently opt out of it.
    if let Err(reason) = policy_permits(ctx, &request) {
        // `IpcResponse::error(code: u32, message: impl Into<String>)`
        // — response.rs:42. Reuse the existing rejection code rather
        // than minting one for a guard that should be rare.
        return IpcResponse::error(ERR_POLICY_FORBIDS, reason);
    }
    match request { /* ... */ }
}

/// Reject a request the active policy forbids.
///
/// Returns `Ok(())` for requests the policy has nothing to say about,
/// which is most of them — the policy constrains transport safety
/// settings, not community operations.
fn policy_permits(ctx: &DaemonContext, request: &IpcRequest) -> Result<(), String> {
    let policy = ctx.policy.read();
    match request {
        IpcRequest::Subscribe { .. } if policy.require_signature_verification => Ok(()),
        _ => Ok(()),
    }
}
```

**The decision this needs, made:** the three fields constrain *transport
safety* (hop count, signature verification, gossip TTL), and the daemon
does not currently expose a command that sets any of them — which is
exactly why nothing reads the policy. So the honest fix is the second
option in the audit: **delete `DaemonContext::policy`, `PolicyReload`,
and `admin::handle_policy_reload`**, and keep policy enforcement in the
CLI where the settings it constrains actually live. If a daemon-side
transport-settings command is added later, the policy check comes back
with it. Storing a value nothing consults is worse than not storing it,
because it reads as enforcement.

### 5.2 `Option<Option<CategoryId>>` is a three-state edit

`GovernanceEntry::ChannelUpdated.category_id` encodes unchanged / clear
/ set as nested `Option`s, while `ExclusionGroupEdit` models exactly
those three states as an enum. It carries a documented `#[allow]` today.

It is a Cap'n Proto union arm, so this is a schema migration:

```rust
// crates/rekindle-types/src/governance/entry.rs
/// Three-state field edit. Same shape as `ExclusionGroupEdit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FieldEdit<T> {
    #[default]
    Unchanged,
    Clear,
    Set(T),
}

// ChannelUpdated
category_id: FieldEdit<CategoryId>,
```

Requires: the capnp schema gains a union for the three cases, both
dispatch halves in `capnp_envelope/governance/` are updated (an unwired
variant **panics at runtime** via the `unreachable!()` guards — types,
merge and codec land in one commit), and `SCHEMA_VERSION` bumps.
**Do this with the next wire change, not on its own** — the bump wipes
DB, Stronghold and Veilid storage together, which is not worth spending
on a type shape alone.

### 5.3 Friend nickname is unwired

`FriendEntry.nickname` exists in the DHT entry, the `friends` table,
`state::friend`, and `friends.store.ts` — set to `None` at every
construction site, with no command to change it.
`rekindle_protocol::dht::friends::update_friend` is the DHT half,
already written and deliberately kept.

Four edits, following `set_nickname` (`commands/status.rs:18`) as the
shape:

```rust
// 1. src-tauri/src/services/friend_runtime/nickname.rs (new)
pub async fn set_friend_nickname_inner(
    state: &Arc<AppState>,
    pool: &DbPool,
    app: &tauri::AppHandle,
    public_key: String,
    nickname: Option<String>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let (pk, ok, nick) = (public_key.clone(), owner_key.clone(), nickname.clone());
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE friends SET nickname = ?3 WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk, nick],
        )?;
        Ok(())
    })
    .await?;

    if let Some(f) = state.friends.write().get_mut(&public_key) {
        f.nickname.clone_from(&nickname);
    }

    // The DHT half. `update_friend` has been sitting unwired for exactly
    // this caller; the friend list is our own record, so this is a
    // plain owner write.
    //
    // There is no `state_helpers::dht_manager` — the established pattern
    // (dht_publish_service.rs:273, profile_push.rs:52) is to take the
    // routing context and build a `DHTManager` around it.
    let dht_target = {
        let node = state.node.read();
        node.as_ref()
            .and_then(|nh| nh.friend_list_dht_key.clone())
            .map(|key| (key, node.as_ref().map(|nh| nh.routing_context.clone())))
    };
    if let Some((key, Some(rc))) = dht_target {
        let dht = rekindle_protocol::dht::DHTManager::new(rc);
        if let Err(e) =
            rekindle_protocol::dht::friends::update_friend(&dht, &key, &public_key, nickname.clone(), None)
                .await
        {
            tracing::warn!(error = %e, "friend nickname DHT write failed");
        }
    }

    crate::event_dispatch::emit_live(app, "friend-event", &serde_json::json!({
        "type": "nicknameChanged",
        "data": { "publicKey": public_key, "nickname": nickname }
    }));
    Ok(())
}

// 2. commands/friends.rs — the #[tauri::command] wrapper
// 3. invoke.rs — register commands::friends::set_friend_nickname
// 4. src/ipc/commands/friends.ts + the buddy-list context menu
```

Note `update_friend`'s `group` parameter stays `None` here — renaming a
friend and moving them between groups are separate user actions, and
the function already treats `None` as "leave alone".

### 5.4 Gossip overlay None race on join

Bug registry #5, still open. `services/community/record_inventory.rs:128`
builds a `CommunityState` with `gossip: None`, and the overlay is only
populated later — `control_membership.rs:67` and
`presence_adapter/gossip_overlay.rs:101` both do
`cs.gossip = Some(...)`. A gossip send between join and the first
presence poll finds `None` and drops.

The registry's own note says "fix with provisional overlay". Concretely:

```rust
// services/community/record_inventory.rs — replace `gossip: None`
// A joined community always has an overlay, even before the first
// presence poll fills it. `None` here meant every send between join
// and that first poll found no overlay and dropped the envelope;
// `GossipOverlay` has a hand-written Default (state/gossip.rs:39) that
// sets `needs_initial_sync: true`, so the poll still knows it has work
// to do — this is not relying on a derived all-zeroes default.
gossip: Some(crate::state::GossipOverlay::default()),
```

Then `control_membership.rs:67`'s `cs.gossip = Some(default())` becomes
redundant and can go, and `gossip_overlay.rs:101`'s assignment stays as
the real population step.

### 5.5 CI has never executed

`frontend-arch`, `security` and `shell` have never run in Actions. This
is the plan's own acceptance criterion from the original dedup plan and
the only one that cannot be verified locally. Push the branch and read
the run. No code change.

## Phase 6 — The roadmap's open items

These were **not** in the first draft of this plan, which claimed to
cover "all outstanding issues" while silently omitting every open
roadmap entry. `docs/roadmap.md` is the project's source of truth and
carries 14 unchecked items; leaving them out made the plan look
finishable when it was not.

They are features rather than alignment defects, so they sit behind
phases 1-5 — but they are what "done" actually means.

| # | roadmap line | what it needs |
|---|---|---|
| 6.1 | PreKey rotation + one-time replenishment (:35) | `replenish_prekeys` exists in `transport/operations/mek.rs`; needs a rotation schedule and a Stronghold-side bundle refresh |
| 6.2 | Game time tracking (:63) | elapsed seconds persisted to SQLite; `GameInfo.elapsed_seconds` already on the wire both tracks |
| 6.3 | Rich presence server info (:64) | `GameInfo.server_address` exists and is unused by the UI |
| 6.4 | Cross-device sync productionisation (:114) | subkey reconciliation; the `sync` command family in 4.2 wave I is the CLI half |
| 6.5 | Push relay mobile testing (:116) | no code — needs real devices |
| 6.6 | Plate Gate stress >1000 members (:117) | `MAX_SEGMENTS × SLOTS_PER_SEGMENT = 2040`; the expansion path is built (`expand_community_segment`) but untested at scale |
| 6.7 | C1-2 lazy per-segment channel records (:118) | deferred segment record creation |
| 6.8 | Community browser / discovery (:122) | no public directory; needs a design decision first |
| 6.9 | Updater wiring (:123, :202) | `check_for_updates` is a stub |
| 6.10 | Connection quality monitoring (:171) | `SharedState` already tracks latency + peer counts from Veilid; needs surfacing |
| 6.11 | Strand Relay 3-hop onion (:190) | single-hop today; §13 |
| 6.12 | Relay capacity advertisement (:191) | bandwidth/latency/uptime; selection uses hash affinity + circuit breaker today |
| 6.13 | In-game overlay (:203) | research |

6.10 is the cheapest real win: the data is already flowing from Veilid
into `SharedState` (`reliable_peer_count`, `live_peer_count`,
`estimated_network_size`, median p75 latency) and nothing renders it.

## Verification

Each phase ends at or above the current baseline:

- `cargo clippy --workspace --all-targets` clean
- `cargo test --workspace` — 1793 passing today, plus the new tests
- `cargo xtask check` 8/8
- `pnpm build` clean
- baseline `xtask/known-duplication.txt` stays empty

Phase-specific:

- **1** — the three `fit_within_budget` tests, and a manual check that a
  channel with 500+ messages actually delivers history.
- **2** — `revoke_reads_route_id_before_delete`.
- **3** — `remove_untracks_the_profile_record`.
- **4.1** — per family, the CLI and the Tauri UI observe the same event
  for the same action. That is the actual acceptance test for
  "frontends are interchangeable".
- **4.2** — per wave, every ported command is drivable from
  `rekindle-cli` end to end.
- **5.5** — a green Actions run. Nothing else proves it.
