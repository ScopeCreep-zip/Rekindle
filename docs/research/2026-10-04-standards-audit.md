# Standards-Based Full Audit — 2026-10-04

Nine parallel audits of the whole project. Unlike the prior audit, each was required to
(1) judge code against primary online standards, (2) inventory every fallback /
legacy-compat pattern (forbidden: the app is unreleased), and (3) correlate with a live
dev-build log. CI was out of scope. Findings below were spot-verified by hand where marked ✔.

Areas: realtime voice/video · Veilid DHT/records · messaging/crypto/wire · presence &
governance · Tauri shell security · storage/vault · frontend · daemon/CLI · Session comparison.

---

## Cross-cutting root causes

1. **Unreliable primary paths patched with durable "fallbacks."** Voice/call signaling is
   fire-and-forget `app_message`; a presence-row "repair" path compensates. DMs are
   `app_message` → 20 retries → dropped; record opens are cancelled by timeouts and then
   "healed" reactively. Standards (JSEP RFC 9429, Trickle ICE RFC 8838 §18.2.3
   "exactly once and in the same order") make the *primary* signaling path reliable instead.
2. **No single owner of shared resources.** DHT record open/close (Veilid close is not
   ref-counted), Tauri events (global `emit` to every window), DM video channels (one per
   peer, every window registers), settings (loaded only in Settings), permissions
   (re-implemented in the frontend).
3. **Sizes and encodings unbounded.** `serde_json` encodes `Vec<u8>` as decimal arrays
   (~3.6 chars/byte); nothing computes Veilid's real per-subkey cap
   `min(32768, 1 MiB / subkey_count)` = **4112 B** for 255-slot SMPL records.
4. **Self-asserted values are trusted.** Lamport clocks (creator election, ordering),
   MEK "election rank," presence timestamps, "plaintext if it parses as JSON," deep links,
   webview-supplied file paths.
5. **Cryptography not spec-conformant.** 1:1 ratchet isn't Double Ratchet; AAD missing on
   several paths; DM-call nonce reuse; group MEK lacks FS/PCS/removal guarantees; daemon
   DMs unencrypted; "SFU" is an MCU that would have to break E2EE.
6. **Two tracks diverged.** Desktop and daemon differ in wire format, crypto, storage and
   features — DMs/friend requests cannot cross tracks.
7. **Secrets at rest under-protected.** Plaintext app DB holding keypairs + all messages;
   weak, unrecorded Argon2 params; daemon "obfuscation" store; device pairing copies the
   identity secret.

---

## Tier 0 — Security (exploitable or data-exposing)

| # | Finding | Location | Standard |
|---|---------|----------|----------|
| S1 | `rekindle.db` is plaintext SQLite (0644): DHT/community owner keypairs, call X25519 secrets, slot seeds, pairing codes, all message bodies + FTS | `src-tauri/src/db.rs:111`, `001_init.sql` | OWASP MASVS-STORAGE-1 |
| S2 ✔ | Daemon sends DMs **unencrypted** (`hex(body)` into a DHT log; signed-only on app_message) | `rekindle-transport/src/broadcast/dm.rs:39-62` | Signal/PQXDH |
| S3 | Desktop receiver accepts any JSON as plaintext; envelope signature lacks domain/recipient → replay to third parties; call signaling plaintext | `message_service/dispatch/prepare.rs:91-96`, `messaging/sender.rs:41` | RFC 5116 |
| S4 ✔ | Governance creator = first entry by self-asserted `(lamport, author)`; lamport 0 (or a lower pseudonym at lamport 1) takes over the community | `rekindle-governance/src/merge/mod.rs:56-73` | Kleppmann BFT-CRDT 2022 |
| S5 | Backdated/forward-forged lamports defeat bans/demotions; `u64::MAX` overflows every peer's clock; logs can be rolled back (shared slot seed) | `merge/*`, `dht_hydration.rs:358`, `lamport.rs:37`, `overflow/mod.rs:394` | Shapiro CRDT; Matrix state-res v2 |
| S6 | Rank rules incomplete (edit owner role, junior unbans owner's ban, `MEKGenerationBump => true`) | `rekindle-governance/src/validate/mod.rs:100-214` | Matrix auth rules |
| S7 | Anyone reaching a member can inject a channel MEK (unsigned, self-declared rank); Kick does not rotate | `rekindle-mek-rotation/src/receive.rs:47`, `convergence.rs:41`, `control_moderation.rs:188` | RFC 9420; Session groups v2 |
| S8 | `core:webview:allow-create-webview-window` for all windows → any window can spawn a `login`/`settings`-labelled window and escalate | `capabilities/default.json:33` | Tauri capabilities docs |
| S9 | `core:default` lets any window forge backend events (deep-link auto-join, fake session reset) | `capabilities/default.json` | Tauri capabilities docs |
| S10 | `upload_attachment`/`download_attachment` accept arbitrary paths (exfiltrate `~/.ssh`, write LaunchAgents) | `commands/community/files.rs:22-61` | OWASP ASVS V12 |
| S11 | `rekindle://` deep link joins a community with no consent; no navigation/new-window guard; peer link-preview URLs unchecked | `deep_links.rs`, `windows.rs`, `MessageBubble.tsx:291` | OWASP Desktop Top 10; Electron checklist §13-14 |
| S12 | DM-call key shared both directions, nonce `seq‖ms` → nonce reuse | `rekindle-calls/src/lib.rs:72`, `transport/packet.rs:34` | RFC 5764 §4.2; RFC 9605 §4.3 |
| S13 | Device pairing copies the identity secret; device pubkey all-zero; unpair can't revoke | `cross_device_sync/pairing.rs:144,173` | Session Protocol V2 |
| S14 | Daemon: Unlock passphrase ignored; macOS disk key ≈ constant; Linux keyutils loses identity on reboot; every CLI client has full authority | `rekindle-node/src/state/keystore.rs`, `dispatch/lifecycle.rs:391`, `connection.rs:101` | keyring docs; threat model A7 |
| S15 | Argon2id at library default (19 MiB, t=2) and params not stored | `rekindle-vault/src/store.rs:160` | RFC 9106 §4 |
| S16 | Identifiers logged at INFO; a `console.warn` logs DM text | `src-tauri/src/lib.rs:56`, `src/actions/chat.actions.ts:92` | OWASP Logging CS |
| S17 | Every window receives every event incl. decrypted plaintext; OS notifications show content | `event_dispatch.rs:139`, `notifications/emit.rs:123` | Tauri events docs |

## Tier 1 — Broken features (live)

| # | Finding | Location |
|---|---------|----------|
| B1 ✔ | Presence row ~6–7 KB vs 4112 B cap → every heartbeat fails from login (live: `failed schema validation …:2` every 60 s, `online_members=0`); failure logged at debug | `rekindle-types/src/presence.rs`, `rekindle-presence/src/community/registry.rs:154,178` |
| B2 | Channel message pages assume 30 KB; real cap 4112 B → sends fail once a page passes ~4 KB | `channel_record/write.rs:13` |
| B3 ✔ | `record not open` (live, 678): channel opens cancelled by our 12 s timeout, never retried; inspect/heal loop only spawned on join, never after login | `dht_hydration.rs:172`, `join/flow.rs:352` |
| B4 | No record ownership: cross-device sync closes its own watched record around every read; leave/remove close shared keys; `None`-writer re-opens downgrade watches | `cross_device_sync/subkey_io.rs:41`, ~70 call sites |
| B5 | Group voice ≥5 members: mixer decodes ciphertext, re-sends with gen 0 (dropped), host deaf | `rekindle-voice/.../mcu_loop.rs:157-218` |
| B6 ✔ | 1:1 ratchet desyncs on loss/reorder/crossing sends → session resets | `rekindle-crypto/src/signal/ratchet.rs:15-30` |
| B7 | No durable offline DMs (20 retries then dropped) | `rekindle-sync/src/pending_retry.rs:15-70` |
| B8 | Desktop ↔ daemon DMs and friend requests incompatible | see crypto/daemon reports |
| B9 | Remote governance changes never reach the UI live; members stripped of roles keep stale role IDs; one change rebuilds all communities | `sync_communities.rs:149-191`, `state_helpers/governance.rs:53` |
| B10 | Cross-window: session-reset prompt races per window; DM video freezes when windows open/close; own DMs never marked own; settings ignored outside Settings | `main.tsx:116-121`, `video_channels.rs:108`, `DmWindow.tsx:63`, `settings.store.ts` |
| B11 | Video/feedback ride gossip (Reliable context, transport retries, double signing); bitrate feedback not stream-scoped (any member can floor your bitrate) | `video_adapter.rs`, `mesh_broadcast.rs:196-270` |
| B12 | Daemon: `friend remove abc` panics dispatch permanently while watchdog stays green; IPC frame reads not cancellation-safe; `VoiceLeave` is a no-op | `social.rs:249`, `ipc/server/connection.rs:136`, `dispatch/presence.rs:136` |
| B13 | Schema-version wipe non-atomic, misses `state/`, `identities/`, `file_cache/` | `db.rs:138`, `setup.rs:280` |
| B14 | Corrupt persisted data coerced to defaults (unknown `friendship_state` → **Accepted**) | `community_loader/friends.rs:24` and ~80 sites |
| B15 | Login window runs post-auth bootstrap that its capability correctly denies | `src/main.tsx:107-118` |

## Tier 2 — Standards-aligned target architecture (one path each)

- **Signaling:** per-peer signaling session over Veilid `app_call` (request/response) with
  session id, sequence numbers, retransmit-until-ack, per-peer state machine and per-peer
  media-ready. Routes exchanged only here. Delete roster-reconcile repair/reciprocity paths.
  (RFC 9429, RFC 8838)
- **Presence/membership:** tiny signed Cap'n Proto liveness beat ≤256 B
  `{pseudonym, global_slot, seq, status, voice_channel_id, expires_at, sig}` with TTL,
  monotonic seq and signed leave; routes/profile in a member-owned "member card" written on
  change only. Encrypt friend presence per friend. (MSC4143/4140/4354, RFC 6121)
- **DHT records:** one record-lease registry (`acquire(key, writer)` → lease; last release
  closes; writer never downgrades; watches and retry policy owned by the registry), plus a
  single `max_subkey_bytes(&schema)` helper bounding every writer; binary encodings.
  (veilid-core 0.5.7; VeilidChat `DHTRecordPool`)
- **Governance:** hash-linked signed op DAG; creator = signer of the pinned genesis hash;
  lamport derived as 1+max(deps); Matrix-style power-ordering + iterative auth checks;
  grow-only local op store with signed checkpoints; strict rank rule.
- **Group crypto:** MLS (RFC 9420) via `openmls` with the DHT as delivery service, commit
  ordering on the governance DAG; channel keys from the MLS exporter; media via SFrame
  (RFC 9605) with per-sender keys rotated on every roster change after a short delay.
- **1:1 crypto:** spec Double Ratchet (`pn`/`n` header as AD, `MAX_SKIP`, skipped keys),
  Sesame session records, signed/one-time prekey rotation with deletion, PQXDH AD =
  IK_A‖IK_B, SPQR implemented from the published spec (libsignal is AGPL). Every DM-class
  payload encrypted; typed envelopes make plaintext unrepresentable.
- **1:1 delivery:** per-pair SMPL inbox / DHTLog carrying ratchet ciphertext as the durable
  path + `app_message` wake-up hint (Track B of `.claude/plans/phase-2-dht-inbox-pivot.md`),
  with a retention bound and receiver-ack truncation. (Session swarm model, per-pair gated)
- **Wire:** one framed Cap'n Proto envelope in `rekindle-transport` used by both tracks;
  append-only ordinals; dead schemas deleted.
- **Media:** forwarding-only relay chosen by measured capacity (never decrypts); one
  low-latency media sender per peer; receiver-driven NACK/PLI to the stream owner; GCC
  (delay-gradient + loss) with audio priority; continuous CBR Opus; per-direction keys.
- **Storage:** per-identity SQLCipher app DB keyed from the vault master; secrets moved into
  typed `VaultKey`s; versioned Argon2id header (64 MiB, t=3, p=4); FKs + transactions;
  strict parsing; `secure_delete`; marker-based atomic reset; daemon uses `rekindle-vault`.
- **Devices:** per-device keys certified by the identity key; revocable.
- **Tauri shell:** drop `allow-create-webview-window`, replace `core:default` with explicit
  grants, file dialogs inside Rust commands, navigation allowlist + deny new windows,
  deep-link confirm, targeted `emit_to`/`emit_filter` routing, isolation pattern,
  `freezePrototype`, real signed updater, cfg-gated dev commands.
- **Frontend:** backend owns retry, permissions, membership, unread, own-message flags,
  preferences (broadcast `preferences-changed`); per-window scoped snapshot command;
  call/notification UI only in buddy-list; WCAG keyboard/dialog fixes; Xfire assets wired.

## Session comparison — summary

Adopt: durable store-and-forward (per-pair gated, not public inbox); rotate on every
removal; per-device keys with revocation; deterministic merge tiebreaks; metadata emitters
off by default; attachment padding + expiry. Do NOT copy: no-PFS stateless 1:1 crypto,
shared device keys, 128-bit seeds, IP-exposing WebRTC calls, server-hosted communities,
deterministic blinded IDs, shared admin master key, unauthenticated inboxes, fast-mode push.
Rekindle currently repeats several mistakes Session already reversed (shared device key,
no rekey on kick, plaintext acceptance, unencrypted presence/profile, unencrypted call
signaling). Rekindle is stronger on identity entropy, receiver anonymity and IP-safe calls.

## Fallback / legacy-compat

More than 100 instances inventoried across the nine reports (presence `media_route_blob` /
`departed` compat skips, `decrypt_*_with_legacy_fallback`, v1/v2 MEK unwrap, `IngressItem::Legacy`,
`LegacyChatEvent`/`isLegacyCommunityEvent`, `open_or_create_record` key rotation on failure,
daemon disk keystore, `to_legacy_u32` RoleId truncation, Stronghold naming, `serde(default)`
for required fields, etc.). Each is deleted as part of the owning Tier 2 rebuild.

## Corrections to prior beliefs

- No Triple Ratchet / SPQR exists in this tree (a saved note claimed otherwise).
- `RecordIndex(remote): Consistency failure` is a veilid-core quirk in the store kept for
  other peers (128-record cap), not an app bug.
- The 2026-10-04 registry-tracking fix in `dht_hydration.rs` is valid but was not the live
  `record not open` cause (B3 is).
- The 2026-10-04 TUI leave-voice fix dispatches correctly, but the daemon handler is a no-op (B12).

## Sources (primary)

RFC 9429 (JSEP), RFC 8838 (Trickle ICE), RFC 8825, RFC 4585, RFC 5764, RFC 6562, RFC 7587,
RFC 7675, RFC 8888, RFC 9605 (SFrame), draft-ietf-rmcat-gcc-02, RFC 9420 + RFC 9750 (MLS),
RFC 5116, RFC 9106, RFC 6121; signal.org/docs (Double Ratchet, PQXDH, Sesame), signal.org/blog
(group calls, SPQR); MatrixRTC MSC4143, MSC4140, MSC4354; spec.matrix.org state-res v2 and room
v10 auth; Kleppmann "Making CRDTs Byzantine Fault Tolerant" (PaPoC 2022); Shapiro et al. INRIA
RR-7506; capnproto.org/language.html; docs.rs/veilid-core/0.5.7 + local veilid-core 0.5.7 source;
VeilidChat `dht_record_pool.dart`; v2.tauri.app (capabilities, calling-frontend, isolation,
updater); Electron security checklist; OWASP MASVS, ASVS, Desktop Top 10, Logging cheat sheet;
sqlite.org pragma docs; zetetic.net SQLCipher design/API; noiseprotocol.org; tokio `select!`
docs; unix(7), pid_namespaces(7); keyring 3.6.3 docs; clig.dev; WCAG 2.2; w3.org WebCodecs;
SolidJS docs; Session whitepaper (arXiv 2002.04609v3), getsession.org protocol/groups/calls
posts, libsession-util, session-storage-server, session-file-server, session-desktop sources,
Soatok critique, Quarkslab audit summary.
