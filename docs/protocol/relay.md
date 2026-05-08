# Relay Protocols — Strand Relay + Mobile Push Relay

Rekindle uses two distinct relay systems for two distinct problems:

- **Strand Relay Network** solves *online but unreachable* — when
  Alice cannot establish a direct route to Bob (NAT churn, route
  expiry, transient connectivity), a mutual friend Carol forwards an
  opaque encrypted blob.
- **Mobile Push Relay** solves *offline mobile platform* — when an iOS
  or Android client is fully suspended and cannot maintain a Veilid
  connection at all, an opt-in third-party relay watches DHT records
  on the device's behalf and sends a content-free FCM/APNs wake.

Neither relay is a server in the traditional sense. Strand Relay is
peer-friends helping peer-friends. Mobile Push Relay is a self-hostable
headless `veilid-server` instance that participates in the same DHT
without holding any privileged keys.

## Strand Relay Network

In Death Stranding terms: relays are *Strands* — connections between
people that become the infrastructure itself. Friends volunteer
bandwidth and route capacity for friends; the network is stronger
because of those connections, not in spite of them.

Implementation: `src-tauri/src/services/relay/{offer,pool,forward,send,presence}.rs`.
The wire protocol uses `MessagePayload` variants in
`rekindle-protocol::messaging::envelope`.

### Roles

```
Alice            Carol (mutual friend, volunteer relay)         Bob
─────            ──────────────────────────────────             ───
                                                                
                       1. Carol creates dedicated relay route
                       ──────────────────────────────────────▶ Bob
                          RelayOffer { relay_route_blob }       │
                                                                ▼
                                                       Bob persists
                                                       offer in his
                                                       relay_offers
                                                       table.
                                                                │
                       2. Bob publishes received offers as a    │
                          padded list in his profile DHT record │
                          (real offers + dummy entries for      │
                          unlinkability).                       ◀
                                                                
3. Direct route to Bob expires
   ▶ pull Bob's profile record
   ▶ pick a random entry
     (opaque blob, padded —
      Alice can't tell which
      friend volunteered)
                                                                
4. RelayEnvelope                                                
   { encrypted_to_bob, target_route }                           
                       ────────▶ Carol receives                
                                  ▶ look up route_id → Bob      
                                  ▶ forward inner payload       
                                  ─────────────────────────────▶
                                                       Bob decrypts
                                                       with his own
                                                       key.
```

### Privacy properties

| Property | Mechanism |
|----------|-----------|
| Alice cannot identify which friend is relaying | Bob's relay pool publishes opaque route blobs padded with dummies. Alice picks a random entry; the route is by definition opaque. |
| Carol does not know who Alice is | The envelope arrives via Alice's private route. Carol sees only `{target_route, encrypted_inner_payload}` plus the route metadata — no sender identity. |
| Carol cannot read the content | The inner payload is encrypted to Bob's identity key. Carol forwards bytes; she does not have the decryption key. |
| Bob cannot link Alice's failed direct attempts to her relay-routed delivery | Different Veilid private routes, different timing, different envelope format. |

### Message types

```rust
MessagePayload::RelayOffer {
    relay_route_blob: Vec<u8>,    // dedicated route Carol just created for Bob
}
MessagePayload::RelayWithdraw {
    relay_route_id: Vec<u8>,      // Carol revokes her offer
}
MessagePayload::RelayAck {
    relay_route_id: Vec<u8>,      // Bob confirms receipt and add to pool
}
MessagePayload::RelayEnvelope {
    target_route: Vec<u8>,        // Bob's route blob (Carol forwards to this)
    inner_payload: Vec<u8>,       // ciphertext encrypted to Bob's key
}
```

### Bob's relay pool publication

Bob's profile DHT record (DFLT, owned by Bob's friend-profile keypair)
includes a relay pool subkey:

```rust
struct RelayPool {
    entries: [Option<Vec<u8>>; POOL_SLOTS],   // POOL_SLOTS = 16
    // unused slots filled with random-looking dummy bytes of the same length
}
```

The dummies are essential — without padding, Alice could count
non-empty slots and learn how many friends volunteered. With padding,
every slot looks identical at the wire level.

### Latency

| Path | Latency | When used |
|------|---------|-----------|
| Direct route alive | 50–150 ms | Default |
| Stale route → strand relay | 60–100 ms | Direct fails; one hop through Carol |
| DHT fallback (no relay) | 200–500 ms | No relay available; Alice writes to Bob's mailbox DHT record |

Relay latency is competitive with direct delivery because Carol's
relay route is itself a Veilid private route — there is no extra DNS
lookup, TCP handshake, or routing decision. The cost is one additional
Veilid hop (Carol's relay path), which is comparable to a single
random DHT routing step.

### Presence caching

Friends acting as relays also serve as a **social CDN for presence**.
When Alice's client wants Bob's current status, it can ask one of
Bob's relay friends via `MessagePayload::StatusRequest`, and Carol
replies with cached presence (what she most recently observed of Bob)
via `StatusResponse`. This is faster than a fresh DHT lookup of Bob's
presence record and uses paths already established for relaying.

## Mobile Push Relay

Mobile platforms cannot maintain persistent Veilid connections. iOS
suspends background processes aggressively; Android is more forgiving
but still kills idle apps. The push relay is a **three-tier escalation**
that gives mobile users timely notifications without breaking the
privacy posture.

### Three tiers

```
┌──────────────────────────────────────────────────────────────┐
│  TIER 1: Foreground Veilid                                    │
│   App is in the foreground. The device runs a full Veilid     │
│   node, receives gossip in real time, generates notifications │
│   locally. No external infrastructure. Indistinguishable from │
│   the desktop app.                                            │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  TIER 2: Background fetch                                     │
│   App is backgrounded but the OS allows periodic execution.   │
│   The app briefly attaches Veilid, calls inspect_dht_record   │
│   on community channel records, fetches new subkey writes,    │
│   decrypts, generates local notifications, and disconnects to │
│   conserve battery.                                           │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  TIER 3: Opt-in push relay                                    │
│   App is fully suspended. A separate headless veilid-server   │
│   watches DHT records on the device's behalf. On change, it   │
│   sends an opaque push via FCM/APNs:                          │
│                                                                │
│       { "t": "wake", "ts": <unix_timestamp> }                 │
│                                                                │
│   Zero message content. Zero metadata about which community,  │
│   channel, or sender. Only "some watched record changed at T."│
│   The OS wakes the app, which then runs the Tier 2 fetch.     │
└──────────────────────────────────────────────────────────────┘
```

The user opts in per identity. Self-hostable: any user can run their
own `rekindle-push-relay` daemon and register their device with it.
The shared default relay (when one exists) holds no privileged keys —
it can only see *that* a record changed, not *what* changed.

### Why "opaque wake"?

A push notification on iOS or Android passes through Apple's or
Google's infrastructure. Even if the relay daemon is honest, the push
*payload* could leak metadata to the platform vendor. Rekindle ships
the wake notification with **no content** — only a timestamp — so
even if Apple/Google read every push, they learn at most "this device
might have had something happen at time T."

The actual fetch happens after the OS wakes the app, *over Veilid*,
where the platform vendor cannot see anything.

### Registration protocol

```
Device                              Relay daemon
──────                              ────────────

▶ register_with_push_relay({
    relay_pseudonym,
    device_push_token,    // FCM token or APNs token
    platform,             // "fcm" / "apns" / "self"
    record_keys: [        // DHT records to watch on my behalf
      community_channel_records,
      mailbox_record,
      personal_sync_record,
      ...
    ],
  })

  ────── MessagePayload::RegisterPushRelay ──────────▶
                                    ▶ persist registration
                                    ▶ open watches on each record_key
                                    ▶ on ValueChange, push opaque wake
                                       via FCM/APNs to device_push_token

▶ unregister_with_push_relay()
  ────── MessagePayload::UnregisterPushRelay ────────▶
                                    ▶ close watches
                                    ▶ delete registration
```

`register_with_push_relay` and `unregister_with_push_relay` are IPC
commands wired in `src-tauri/src/services/push_relay.rs`. The local
device persists each registration in the `push_relay_registrations`
SQLite table so it can re-establish or revoke registrations on next
launch.

### Wake-notify debounce

When the device is in `Tier 1` foreground mode, it doesn't need wake
pushes. The relay receives `WakeNotify` heartbeats from the device
indicating "I'm alive, don't bother sending pushes for the next N
seconds" — see `last_wake_notify_secs` in `AppState`. This avoids
duplicate notification pathways (gossip + wake) firing simultaneously.

### Security posture

The relay daemon:

- **Sees** only that one or more of the registered record keys had a
  subkey change.
- **Does not see** the content of any change (DHT record contents are
  encrypted at the application layer with MEK or Signal — the relay
  has no decryption keys).
- **Does not see** which subkey changed if the daemon registers a
  range-watch (Veilid notifies on `ValueSubkeyRangeSet` so the daemon
  may know "subkey 7 changed" — this is metadata, but not content).
- **Does not see** the sender of any change — the writer's identity is
  opaque to the daemon.
- **Could** correlate device push tokens with the watched record keys
  it was given. This is a real metadata risk and is the reason push
  relay is **opt-in**, **self-hostable**, and **never on by default**.

The threat model trades some metadata (the relay knows which DHT keys
this device cares about) for the ability to receive timely
notifications when fully suspended. Users who consider this trade
unacceptable can disable Tier 3 entirely and live with Tier 2's
periodic background fetch.

## Where to look

| Concern | File |
|---------|------|
| **Strand Relay**: volunteer / revoke an offer | `src-tauri/src/services/relay/offer.rs` |
| **Strand Relay**: received-offer pool persistence | `src-tauri/src/services/relay/pool.rs` |
| **Strand Relay**: forward an envelope | `src-tauri/src/services/relay/forward.rs` |
| **Strand Relay**: send via relay (Alice's side) | `src-tauri/src/services/relay/send.rs` |
| **Strand Relay**: presence cache (`StatusRequest`/`StatusResponse`) | `src-tauri/src/services/relay/presence.rs` |
| **Strand Relay** wire types (`RelayOffer/Withdraw/Ack/Envelope`) | `crates/rekindle-protocol/src/messaging/envelope.rs` |
| **Push Relay**: client-side registration | `src-tauri/src/services/push_relay.rs` |
| **Push Relay** wire types (`RegisterPushRelay`, `UnregisterPushRelay`, `WakeNotify`) | `crates/rekindle-protocol/src/messaging/envelope.rs` |
| **Push Relay** SQLite schema | `src-tauri/migrations/001_init.sql` (`push_relay_registrations` table) |
| Reliability tracking for gossip ziplines | `services/community/` (peer reliability dirty set, flushed every 30 s) |
