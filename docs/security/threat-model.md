# Threat Model

This document is the canonical, structured threat model for Rekindle.
It enumerates **adversaries** (who we defend against), **assets**
(what we protect), **threats** (what could go wrong), and the
**mitigations** in the architecture that address them. Threats are
classified using **STRIDE** for traditional security and **LINDDUN**
for privacy properties.

It is read alongside:

- [`overview.md`](overview.md) — five-layer encryption stack
- [`crypto-primitives.md`](crypto-primitives.md) — primitive choices and
  rejected alternatives
- [`privacy-properties.md`](privacy-properties.md) — what Veilid gives
  vs. what Rekindle adds
- [`../../SECURITY.md`](../../SECURITY.md) — disclosure policy

## Scope and intended users

Rekindle is built for **vulnerable users**: privacy-conscious
individuals, activists, journalists, marginalized communities,
researchers, dissidents, and anyone who would be materially harmed by
metadata collection or content disclosure. The threat model reflects
that posture: we accept usability tradeoffs to close attack paths that
would matter to those users.

Quoting the project's standing constraint: **every fallback or
auto-recovery path is an attack surface.** We refuse plaintext
fallbacks, auto-rehandshake without user consent, smart trust
propagation, spoofable display fields, deterministic timing, and
silent telemetry. This document explains why each of those is treated
as load-bearing.

## 1. Adversaries

| # | Adversary | Capabilities | Realistic for Rekindle? |
|---|-----------|--------------|-------------------------|
| A1 | Passive network observer (ISP, transit) | See all packets in transit; correlate timing and volume | Yes — assumed |
| A2 | Active network observer | Drop, inject, replay, MITM transit packets; tamper with TLS where used | Yes — assumed |
| A3 | Curious peer (community member) | See everything available to a community member: gossip, SMPL writes, registry, presence routes | Yes — by design |
| A4 | Banned / departed peer | Previously had legitimate access; retains historical material; can write to own subkey indefinitely | Yes — handled |
| A5 | Malicious community admin | Has elevated permissions (`MANAGE_*`, `BAN_MEMBERS`); cannot decrypt content; can poison governance | Yes |
| A6 | Compromised device (post-unlock) | Full RAM read, key extraction, live observation | Yes — partial mitigation |
| A7 | Device seizure (locked) | At-rest disk access; no live secrets | Yes — handled |
| A8 | Coerced user | Forced to unlock the device or hand over keys | Partial — we cannot fully mitigate, but we minimise damage |
| A9 | Platform vendor | iOS / Android / Windows / macOS OS-level access; push notification provider (Apple, Google) | Yes — minimised by opt-in |
| A10 | State-level mass surveillance | Bulk traffic capture; partial Veilid relay node operation; subpoena power against centralised services | Yes — assumed |
| A11 | Nation-state targeted attack | Resources to compromise specific peers, exploit upstream dependencies, coerce relay operators | Partial — best-effort |
| A12 | Supply-chain attacker | Compromised dependency, malicious build, registry confusion, lockfile tampering | Yes — handled by build hygiene |

We **do not** defend against:

- An adversary with full physical access to an unlocked, logged-in
  device. Once the user has authenticated and the vault is open, the
  secrets are in memory and the adversary has them.
- An adversary who controls the user's identity-key generator at the
  time of identity creation. We assume the OS CSPRNG is not backdoored.
- A targeted-malware compromise of the user's device that lies dormant
  before our process starts. The OS is the trust root; if it is
  compromised, we cannot save the user.

## 2. Assets

| # | Asset | Sensitivity | Where it lives |
|---|-------|-------------|----------------|
| Z1 | Identity keypair (Ed25519) | Critical — root of all derived keys | Vault (`rekindle-vault`) on disk |
| Z2 | Per-community pseudonym (Ed25519) | Critical for unlinkability | Derived per-session via HKDF; cached in memory |
| Z3 | Signal Protocol session state | High | Per-identity SQLCipher DB, `ratchet_sessions` (one `SessionRecord` per peer device, plan E2.3) |
| Z4 | Pre-keys + signed prekey | Medium | Secrets in the SQLCipher DB (`signal_prekeys`); public halves in the per-device DFLT(101) prekey record (DHT), whose owner keypair is in the vault (plan E2.4) |
| Z5 | Channel MEK (per channel, current generation) | High | Memory cache; historical generations in the vault |
| Z6 | DM MEK chain | High | Vault |
| Z7 | Slot keypair (per community) | Low — derived deterministically; not sensitive | Derived from `slot_seed` on demand |
| Z8 | Slot seed | High — distributed via invite | Vault |
| Z9 | Master secret (cross-device) | Critical | Vault; transferred only via paired-device handshake |
| Z10 | SQLite contents (messages, friends, communities, etc.) | High — represents social graph | `state/{pk}.db`, SQLCipher keyed from the passphrase (Argon2id, then BLAKE3 `derive_key` with `VAULT_APP_DB_KEY`); unreadable before unlock |
| Z11 | Local Veilid node identity | Medium | Veilid storage; relinking requires bootstrap |
| Z12 | Friend list, community membership | High — represents who the user knows | DHT (encrypted) + SQLite; `state/{pk}.files/` directory names are keyed hashes, never ids (plan E3.0a), community pictures included (plan E3.1) |
| Z13 | Presence (status, route blob, game info) | Medium | DHT presence record (visible to friends/community members). **Audience gap (2026-11-18 audit, plan E3.2 S7c.8 (4)):** a member's routes subkey (`M`/6) is readable by anyone holding its member record key, which `MemberJoined` and the checkpoint directory pages name: past, kicked and banned members and I16 outsiders, wider than "community members". With today's 1-hop inbound routes each route blob exposes its entry hop's IP (L2); closed by Phase RH's 3-hop inbound routes, after which a blob names no hop that knows our IP. |
| Z14 | Plaintext message content | Critical | Memory only; never persisted unencrypted |
| Z15 | Audit MAC key + tail anchor | High — tamper-evidence root | Vault; loss invalidates the chain |
| Z16 | Device login skin | Low — plaintext by design | `data/skins/login.json` and at most one stripped, render-only `<hash>.rkskin` (no name, description, source, author key, signature or sounds; re-hashed, so it matches no shared package); outside `state/`, so a schema reset keeps it (plan E7.7.2) |

## 3. STRIDE Threats and Mitigations

### S — Spoofing

| ID | Threat | Mitigation |
|----|--------|------------|
| S1 | Active attacker impersonates a friend in a 1:1 chat | Signal Protocol X3DH binds session to identity-key signatures; trust-on-first-use (TOFU) is the default with optional out-of-band verification. Identity-key changes surface a re-verification prompt, never auto-accept. |
| S2 | Banned member writes to their SMPL subkey claiming to be admin | Reader-validates: every reader recomputes `effective_permissions(writer, channel)` from the merged CRDT state. A writer without `MANAGE_*` permission has their entry silently dropped. (See [`../architecture/communities.md` §9](../architecture/communities.md#9-permissions).) |
| S3 | Attacker sends gossip envelopes claiming to be from another sender | Every envelope is Ed25519-signed by the sender's pseudonym. Recipients verify before processing. Forged signatures fail verification and are dropped. |
| S4 | Attacker spoofs the daemon over the IPC bus | Noise IK handshake binds the connection to the daemon's pre-known static key. The socket is owner-only (`0700` directory, `0600` socket; owner-only DACL on Windows), a peer whose UID differs from the daemon's is refused (`SO_PEERCRED`; `getpeereid` on macOS), and both UIDs plus the socket path are in the Noise prologue, so a handshake relayed to another user's or another path's bus fails. The boundary is the OS user: a process running as the same user is trusted to connect, and its clearance comes from the registry (plan D1). |
| S5 | Attacker spoofs an MEK-rotation `MEKGenerationBump` | Reader-validates the rotator: `MEKGenerationBump` carries `trigger_departed` and `cascade_skipped`; every reader recomputes the deterministic rotator and checks the writer matches. Forged bumps are silently dropped. |
| S6 | A holder of a stolen master secret (Z9) retires a member's honest device keys and endorses its own in a community | Not made worse by per-device op keys (ADR 0022, Proposed; plan E3.2 S7d-keys), and not helped: the master secret derives the community pseudonym that endorses device keys, so its holder can do this as it can impersonate today (A6, A8). A device holding the master secret is the user (ADR 0022 §0, corrected 2026-10-24, prod research batch 7): device keys keep an honest restore from tripping D15 but do not stop theft. Every change to a member's effective device set shows a "devices changed" notice to other members, the only signal; the remedy for a stolen device is an identity reset (a device that is not cross-signed is not trusted, Matrix MSC4153; the reset of the cross-signing identity, Matrix MSC4312), not an existing-device requirement (Keybase), which would break restoring a lost sole device from the seed. ADR 0022 rule 7's own-key re-admission is not theft protection either, since a thief holding the master secret satisfies it; theft recovery is rule 0's identity reset. Retiring a key does not retire the keys it co-signed (Keybase revokes only the named key), so a co-signee of a key found compromised needs its own retirement or a moderator's review, and the UI lists those co-signees (amended 2026-10-24, prod research batch 10). |

### T — Tampering

| ID | Threat | Mitigation |
|----|--------|------------|
| T1 | Network adversary modifies a message in transit | Layer 1 Veilid transport encryption (XChaCha20-Poly1305) provides AEAD; tampering breaks decryption. Layer 3 Ed25519 signature on every envelope detects sender-end tampering. Layer 4 AES-256-GCM AEAD with content-bound AAD detects ciphertext tampering before decryption. |
| T2 | Curious peer modifies a stored chunk in their cache and serves the modified version | Layer 4 per-chunk SHA-256 verification on receive. Mismatches are dropped and the chunk is re-fetched from a different peer (swarm fetch). |
| T3 | Attacker modifies an `AttachmentOffer` in transit | Offer is signed by sender. Recipients additionally recompute the flat-list Merkle root over `chunk_hashes` and compare against the announced `merkle_root` to defend against serialization-confusion attacks. |
| T4 | Compromised peer writes garbage to a subkey to corrupt the CRDT view | Writes only affect the writer's own subkey. Garbage entries fail validation (`validate.rs`) or permission checks and are dropped. The CRDT merge is deterministic; honest peers converge regardless of what one peer wrote. |
| T5 | DHT routing node tampers with stored values | Veilid stores values across multiple replicas; Veilid's transport AEAD covers writes. Application-layer signing and AEAD provide the actual integrity guarantee. |
| T6 | Supply-chain attack via tampered dependency | `Cargo.lock` pinning, hash verification on `cargo install`, no `[patch.crates-io]` overrides for security-critical crates. Reproducible-builds doc tracks deterministic build verification (deferred — see open work). |
| T7 | A restored backup, OS snapshot or copied data root (I14), or a forced reuse of a device key, signs two different governance ops at one seq | ADR 0022 (Proposed; plan E3.2 S7d-keys): ops are signed by per-community device keys the community pseudonym endorses, in a grow-only set keyed by (device key, incarnation), never a seq-positioned log, so the endorser cannot fork. The device key and its seq head are never in a Rekindle backup or export (they live in a separate device vault that identity export never reads; corrected 2026-10-24, premise check), and since a snapshot restore cannot be prevented, every unlock runs a barrier before the first governance write: it reads the device's own `GovHead` from the DHT and asks every ring peer it knows, taking the maximum over every peer that answers within one app-call timeout (the bound of one round; a round with no answer is retried) of the highest seq of that device reachable from its frontier tips (the DHT read alone only when the member knows no ring peer at all, as for a sole creator; known ring peers that do not answer leave governance writes pending with a visible state, so a fork does not reopen when every ring peer is offline), and signs nothing until the local head is at least every result; a restored device retires its key at the barrier's maximum (corrected 2026-10-24, premise check) and mints a new one, so no fork happens, and the restore counts as the member's first fork; the re-key is a TrustChanging intent the user commits, never automatic. With a copied data root (amended 2026-10-24, prod research batch 10) each copy detects the clone itself: an op signed by its own device key that is not in its local signed log makes it retire that key at its own local head (each device watches its own record for this, corrected 2026-10-24); every device also checks for a retirement of its own key before each sign and on every member-set change, stops signing and shows "this device's key was retired by another copy of this data". The retirement holds for every copy, so no copy keeps the key and none can displace the other (Signal's 4409 ping-pong, Tailscale's last-writer flapping); each copy re-enrols with a fresh key and replaces every copied per-device secret (per-community device keys, `K_M(p,d)`, DM and ratchet device state, the sync device state), while the master secret and pseudonyms stay shared. Readers never prefer the last writer: with several retirements of one key the lowest `lastSeq` counts (a retirement never reaches below the highest seq of that key that another key's op builds on or that its carrier's causal past holds, so nobody erases history others built on, and a late op building on a quarantined range moves just those ops back to applied, shown; *Corrected 2026-10-24 (pass 3)*, from a floor over the carrier's causal past alone, which a carrier with genesis-only deps bypassed), same-seq pairs at or below it are shown as equivocation, and ops above it from either copy are quarantined and shown, never dropped silently, with a "re-send under new key" action for their author. Sealing the op key in the OS keystore so a copied data root fails closed is a platform lead (plan E5.3); the same-machine instance lock is a separate guard, not the defence. A fork that happens anyway retires that device key only: ops below the fork seq stay valid, the conflicting ops and later ones are quarantined, and the person is not muted. Retirement is permanent, and a late retirement only moves ops from applied to quarantined, shown; quarantined ops stay in the DAG as ancestry, so their dependants resolve, and an op whose authority came from a quarantined op is re-judged at its own causal position (the power state in its causal past, plan E3.2 S7c.3a; decided from research 2026-10-24, batch 14) and quarantined, shown with its cause, only if its authority fails (corrected 2026-10-24, pass 2), with the lowest `(lamport, op_hash)` quarantined op in its authority path as the cause; a refusal caused by a power op that is later quarantined, a ban or kick among them, stays until its author re-sends the op (*Corrected 2026-10-24 (pass 3)*). A re-admitted key's ops count only with the re-admission in their causal deps. Every signature binds the community; the endorsement binds member record, device key, incarnation, the device record and the fork evidence it answers, the retirement member record, device key, incarnation and `lastSeq` (it answers no fork: it is what opens one), the re-admission member record, new key, incarnation and fork evidence, and the pseudonym-signed successor the cut, the new anchor and the fork evidence (corrected 2026-10-24, premise check: it read "every signature binds community, member record, device key, incarnation and fork evidence", which the ADR 0022 rule 10 layouts do not); each endorsement also binds the hash of the endorsement set it extends, so its validity is the same in every carrier and nothing replays across communities or forks. |
| T8 | A colluding majority endorses a false checkpoint snapshot, or two digests for one cut, so a joiner starts from a wrong state | Plan E3.2 S7c (*Corrected 2026-10-24 (pass 3)*): a checkpoint is an acceleration hint, never authoritative; no op is quarantined, rejected or skipped for being outside a cut. A joiner checks the snapshot against the endorsed `stateRoot`, skips fetching only the ops in the cut index, and merges every other op through a resumable merge that returns a result only when it equals the full merge (*Decided from research 2026-10-24 (batch 14):* every op is judged against the power state in its own causal past, so a later power op re-judges no earlier op; the result is exact when every later power op has the whole cut in its causal past, none is a device op, every later kick or ban's `targetSeqFrontier` covers its target's last seq in the cut, none forks into the cut and none sorts before it; it read "every later op has the whole cut in its causal past, none is a power or device op"), otherwise it uses an older hint or replays in full. After a resume its D15 roster checksum is compared with ring peers that replayed in full, and a mismatch drops the hint. Two digests with strict majorities for one cut are provable equivocation: the endorsing ops are quarantined and shown, their double endorsers flagged, and readers use no hint for that community. Ops are never pruned, so full replay is always possible. |

### R — Repudiation

| ID | Threat | Mitigation |
|----|--------|------------|
| R1 | A peer denies having sent a message | Every gossip envelope is signed by the sender's pseudonym key. Within a community, signatures are non-repudiable to that pseudonym. |
| R2 | A peer claims someone else sent a message they actually sent | Same as R1 — signature verifies against the sender's pseudonym, which is bound to a specific subkey by the SMPL schema. A peer cannot forge a signature claiming to be another peer. |

We deliberately do **not** add a community-wide signing chain or
notarisation. Pseudonym-level non-repudiation is sufficient; adding more
would create cross-community linkability.

### I — Information Disclosure

| ID | Threat | Mitigation |
|----|--------|------------|
| I1 | Network adversary reads message content | Layer 4 (per-channel MEK / Signal Protocol). Content is never sent in plaintext. |
| I2 | Curious community member decrypts a channel they don't have access to | Channel MEK is wrapped per-recipient at distribution. A peer who never received the wrap cannot decrypt. |
| I3 | Banned member decrypts post-ban messages | MEK rotation on member departure. New MEK is wrapped only for remaining members. From plan E3.5 the rotation is a write-once ballot used only after a strict majority of eligible rotators confirm it (ADR 0018); until it is confirmed, the removed member can still read new text under the old key. |
| I4 | Banned member decrypts pre-ban messages they had once cached | Acknowledged limitation. Forward secrecy protects only future content. The MEK rotation cadence and the deterministic rotator protocol minimise the exposure window for *future* content. |
| I5 | Veilid relay node sees content | Layer 1 Veilid AEAD makes intermediate relays see only ciphertext. Layer 4 application AEAD makes them see only outer-envelope ciphertext even after Layer 1 unwraps. Defense in depth. |
| I6 | Attacker reads at-rest secrets from disk | Layer 4: `rekindle-vault` SQLCipher double-encryption — page-level AES-256-CBC keyed by Argon2id-derived master, per-entry AES-256-GCM on top, sealed when not in use. The identity database `state/{pk}.db` is SQLCipher as well, keyed from the same Argon2id master (`VAULT_APP_DB_KEY`), so it cannot be read before unlock (plan D1). Ratchet sessions and prekey secrets (`ratchet_sessions`, `signal_prekeys`) are single-layer by design (plan D3): SQLCipher's page encryption is their only at-rest layer, because a per-entry seal under a key from the same unlocked master adds nothing — against A7 neither key exists before unlock, and against A6 both are in memory. Signal Desktop, Signal Android and Wire store sessions the same way. What stays readable before unlock is listed, with its reason, in the pre-unlock plaintext inventory below. |
| I7 | Memory dump on running app reveals secrets | Limited mitigation: every secret type implements `Zeroize + ZeroizeOnDrop` so secrets are wiped when the holding struct drops. Live secrets in active use are still in memory and recoverable from a memory dump. |
| I8 | Attacker correlates a user's pseudonyms across communities | Pseudonyms are derived per-community via HKDF(`master_secret`, `community_id`). Distinct communities yield cryptographically unrelated pseudonyms. An attacker who compromises one community's member list learns only pseudonyms meaningless outside that community. |
| I9 | Push relay correlates wake-pushes with content | Wake payload contains only `{ "t": "wake", "ts" }` — no content, no record key, no community ID. Relay cannot correlate; platform vendor cannot correlate. The relay daemon does see *which* DHT keys the device cares about, which is metadata; this is opt-in and self-hostable. The target (plan E5.2, E5.6) registers under a per-relay `device_relay_key`, never the identity key, names opaque watch tokens instead of raw record keys, lets a registration expire unless it is re-signed within 14 days, and drops the timestamp, so the wake becomes `{ "t": "wake" }`. (See [`../protocol/relay.md`](../protocol/relay.md).) |
| I10 | Game detection leaks "what the user is playing" | Detection is opt-in per identity. Database is bundled (no remote lookup). Output goes only to friends granted presence visibility. Per-friend-group filters ship in the protocol but not yet in the UI. |
| I11 | A coerced user is forced to reveal their pseudonym in another community | Cross-community linkability requires the user to either share the master secret or sign a challenge with two distinct pseudonyms. We provide no API that does this implicitly; OAuth2 / connected-account features that would create this correlation are deliberately omitted. |
| I12 | Address-book matching reveals friend lists to a server | We do not implement address-book matching. Identity is a keypair, not a phone number. There is no server. |
| I13 | A call participant, or someone who later seizes a participant's device, reads call media they should not | Call media uses sender-owned SFrame keys (RFC 9605 §5.1, ADR 0016), delivered pairwise and never under a MEK. **Leaver:** each sender starts a new key generation when someone leaves, used after 5 s, with at most one rotation pending; a leaver cannot read media sent more than ~10 s after leaving (RingRTC's bound under churn), and a kicked or banned member always triggers it, stage channels included. **Joiner:** a join ratchets the sender's key one step (RFC 9605 §5.1) and the joiner gets only the new step, so it cannot read media from before it joined. **Device seizure:** media keys are sealed (HPKE Base) to a per-session X25519 key erased at session end, so recorded calls stay sealed after a later seizure (RFC 9180 §9.1). All three landed in plan step C7.29. Accepted limits: no in-session post-compromise security and no periodic rotation (as DAVE and RingRTC); a > 4 participant MCU host decodes and hears plaintext until E4.4's forwarding-only relay. DM calls already use a fresh ephemeral X25519 per call. |
| I14 | Ratchet state is rolled back or cloned — power loss on macOS before a write reaches the disk, a restored backup, a VM snapshot, a copied data root — so message keys are used twice and consumed one-time prekeys come back | The Double Ratchet cipher is DR rev. 4 §7.2's AES-256-CBC + HMAC-SHA256, so a reused message key shows only whether two plaintexts share leading blocks, never their XOR or an authentication key ([`crypto-primitives.md`](crypto-primitives.md) §5). The identity database runs `synchronous=FULL` with `fullfsync` and `checkpoint_fullfsync` (macOS `F_FULLFSYNC`), so a committed ratchet step survives a power loss before the message it keys is sent; every erasure of key material counts, a deleted row (session, prekey secret, group sender key or chain, presence key) and an eviction inside a record blob alike (a ratchet step's spent chain and skipped keys, the E2.5h archived-session cap, a delete-then-insert; RV5 L2): it marks its connection erased (`rekindle_db::erase`; `cargo xtask check-key-erasure` keeps every key-material `DELETE` to `erase::delete`), and after the outermost commit a `wal_checkpoint(TRUNCATE)` with `journal_size_limit=0` removes the old frames from the `-wal` file. **Bounded window:** the truncation is queued on the identity's database thread directly behind the calls already waiting when the erasing call returns, never inside that call (so no call's answer waits on it, RV5 M4), and it copies at most `wal_autocheckpoint` (1000 pages) of frames; erased key material is recoverable from the `-wal` file only until those queued calls finish, the next call on that database runs after it (test `erase::tests::an_erase_leaves_the_wal_by_the_next_call`), and a close truncates too. The mark is per connection, one identity per database file (RV5 L3) (`rekindle-db/src/open.rs`, `rekindle-db/src/erase.rs`). Session keys are erased when a session leaves the archive (at most 3 inactive sessions per peer device, plan E2.5h; the record write that evicts it marks the erasure like any other) and on an identity reset. *Superseded 2026-10-24 (plan E2.5h):* ending a friendship no longer erases its sessions; a removal, a block, a cancelled request and a re-add keep them, so a re-friended peer's accept replaces the active session instead of being refused by a kept one. `state/` (every identity's database, vault and Veilid storage) is excluded from OS backups at every start, after the layout check (plan E7.1, `rekindle-utils/src/paths/backup.rs`, called from `rekindle_node::host::claim`): on macOS Time Machine's sticky exclusion (`com.apple.metadata:com_apple_backup_excludeItem`, what `NSURLIsExcludedFromBackupKey` and `tmutil addexclusion` set), on Linux Déjà Dup's `.deja-dup-ignore` marker; Windows has no per-user mark (the VSS `FilesNotToBackup`/`FilesNotToSnapshot` keys are `HKLM`, plan F3's installer). **Restore case:** the mark keeps honest backup tools from copying the ratchet, not a VM snapshot, a whole-disk image, a backup tool that ignores the mark or a manual copy of the data root; such a restore is the rollback above, with the same documented limit. **Documented limit:** after a rollback of k messages, the next k messages the peer receives from the restored side are lost silently (their keys were already used). Governance op logs face the same rollback; ADR 0022's unlock barrier (row T7) covers them, since the restore itself cannot be prevented. |
| I15 | A seizer of a locked device (A7) reads which skin the login screen shows, or a coercer (A8) makes the user set one; a hostile package or the `skin://` scheme reaches more than the skin store | Before unlock only the device login skin (Z16) exists: a built-in, or a stripped copy that the user chose explicitly (a non-built-in login skin is a TrustChanging intent whose review says it will be visible without the passphrase, `Escalation::VisibleBeforeUnlock`; nothing is copied automatically). The identity's choice and library stay in SQLCipher. The `skin://` scheme serves only bytes the Host returns for a content address (`IpcRequest::SkinFile`, `rekindle_skin::serve`): one package address and one file name (`tokens.css` or `<blake3>.png|gif`), any other path refused before a lookup; a library package only while unlocked; the login copy only if its bytes hash to its name. Every package passed `rekindle-skin-helper`, and the stylesheet is emitted from parsed numbers and registry names only. Built-ins go out `Cache-Control: immutable`, everything else `no-store`, so the webview's disk cache never holds a library pack across a lock. The CSP adds `skin: http://skin.localhost` to `img-src`, `style-src` and `connect-src` only; no `asset:` protocol, no remote origin. |
| I16 | A party that saw an invite's `InviteCreated` op (any member, or anyone who read the op log) reads the community's current governance state (roster, roles, channels, bans) before it joins, without the invite code | Plan E3.2 S7c.8 (2), corrected by the 2026-11-18 audit: a pre-join checkpoint hint or snapshot read (`checkpointHintRequest`, `checkpointStateRequest` with `inviteCodeHash`) must prove the invite code, not name its public `code_hash`. The request carries `inviteTag`, BLAKE3 keyed under `derive_key("rekindle-invite-read-tag-v1", code)` over its signed bytes (community, what is asked, requester, code hash; `rekindle_secrets::invite::pre_join_read_tag`), and the responding inviter checks it under its own stored copy of the code (`community_invites.code`; the op log carries only the hash). A Veilid `app_call` names no sender (veilid-core 0.5.8 `rpc_processor/rpc_app_call.rs:102-114`) and the requester pseudonym is self-chosen, so the former per-invite count of distinct requesters bounded nothing: an outsider could spend its `max_uses` places before the real joiners. It is removed; the read is also served only for a live invite (`GovernanceState::live_invite`) to an unbanned requester, and metered per invite at the gossip floor. Joining needs the code too (plan E3.2 S10, corrected from the joiner's own signature over the public `code_hash`, which any reader of the op log could make while the invite had uses left): `MemberJoined.invite_proof` is signed by the invite's join key, seeded by `derive_key("rekindle-invite-join-v1", code)`, whose public half is the `code_hash` every reader verifies it under, over the community, the joiner pseudonym, its record and the hash, so it is neither forgeable without the code nor replayable for another joiner (`rekindle_governance::op::check_invite_proof`). Accepted: a holder of the code learns the state a joiner learns, which the code already grants by letting it join (§5). |

### Pre-unlock plaintext inventory (A7)

Every file a seizer of a locked device (A7) can read without the passphrase, and why it is not
encrypted. This list is kept current: a step that adds a plaintext file adds its row here.
Paths are under `DataRoot` (`crates/rekindle-utils/src/paths.rs`); `state/` is `DataRoot.data/state`.

| What | Readable before unlock | Why it is plaintext |
|------|------------------------|---------------------|
| `state/identities.json` | Public key, display name and creation time of every identity on the device | The login picker and `IdentityList` list identities before any unlock (`rekindle-db/src/registry.rs`) |
| `state/layout-version` | The `SCHEMA_VERSION` the root was laid out for | The reset trigger must be read before any SQLCipher database can be (`rekindle-db/src/reset.rs`) |
| `state/{pk}.vault.hdr` | Magic, version, Argon2id parameters and salt | Needed to derive the vault key; none of it is secret, only unique per vault (`rekindle-vault/src/header.rs`) |
| `state/{pk}.vault`, `state/{pk}.db` | Size and timestamps only (SQLCipher) | — |
| `state/{pk}.files/` | The tree's shape: how many scope directories, attachment directories and chunks, their sizes and timestamps. Every directory name is a 32-hex keyed BLAKE3 hash under a key derived from the identity secret (`FILE_CACHE_NAMES`), so no name is a community id, conversation id, peer key or attachment id, and names are unlinkable across identities; chunks are FEK-encrypted | Stable names let the cache be re-found after a restart without a database lookup (plan E3.0a) |
| `state/veilid/` | Veilid's table store and its ProtectedStore. The ProtectedStore is file-backed and Rekindle sets no device-encryption password, so the table-store device key is on disk: the node identity (Z11), route secrets and the record tables (what the record tables reveal, such as which records were opened, is not yet traced) | `always_use_insecure_storage` defaults on because the OS keyring crate panicked on Linux (`rekindle-types/src/config/mod.rs`); the store holds Veilid's secrets, never Rekindle's identity keys |
| `data/node.lock` | That a node used this root | One node per data root (`rekindle-db/src/lock.rs`) |
| `data/file_cache/` | Nothing on a current install: nothing writes it since plan E3.1. An install from before keeps chunk directories and community pictures there, named by community id, until the desktop's next start removes the directory (`rekindle_db::reset::prune_file_cache`) | The layout before E3.1; a reset removes it as well |
| `config/config.toml` | The user's settings layer | Read before unlock to start the host |
| `config/agent-keys/` | Each CLI/TUI agent's Noise static keypair (`<agent>.pub`, `<agent>.key` with the private key `0600`) and its integrity checksum (`<agent>.checksum`) | A client's key must survive a reboot because the daemon remembers it; the boundary is the OS user (S4) |
| Logs | Scrubbed lines: identifiers are per-process keyed-hash tags (W8) | Diagnostics |
| `$XDG_RUNTIME_DIR/rekindle/bus.pub` | The bus's public key (the private key never reaches the disk) | Clients read it to connect (`rekindle-ipc/src/noise_keys.rs`) |
| `tui.cursor` in the IPC runtime directory (`rekindle_ipc::runtime_dir()`: `$XDG_RUNTIME_DIR/rekindle/` on Linux, `~/Library/Application Support/rekindle/` on macOS, `%LOCALAPPDATA%\rekindle` on Windows) | The TUI's last event-stream cursor. On macOS and Windows the directory is persistent, not a per-session runtime directory | The TUI resumes its event stream from it after a restart (`rekindle-tui/src/app/stream.rs`) |
| Webview data | Whatever the webview engine persists: on macOS under `~/Library/WebKit/<bundle id>/WebsiteData`, on Linux and Windows under the app's `app_local_data_dir`. Rekindle writes `sessionStorage` only (the event cursor), but nothing enforces that no other storage is written | The webview engine owns the store |

| `data/skins/` (Z16) | `login.json` (`{skin}`: a built-in id or `dev:<hash>`) and at most one stripped, render-only package: the login screen's colours and region images, nothing that names the skin or its author | The login window shows it before any unlock (plan E7.7.2); it survives schema resets (`rekindle-db/src/reset.rs`) |

### D — Denial of Service

| ID | Threat | Mitigation |
|----|--------|------------|
| D1 | Spammer floods a community channel with messages | Per-sender rate limit (token bucket, default 10 msg/s). Excess messages are dropped at the gossip stage and not re-broadcast. Governance can adjust the threshold via `AutoModRule`. |
| D2 | Banned member continues writing to their subkey | The CRDT merge drops their entries. The wasted bytes are local-only (their subkey storage, not anyone else's). |
| D3 | Attacker floods the daemon's IPC bus | Per-connection rate limit on `rekindle-node` (100 req/s, refill bucket). Handshake DoS protection (5 s timeout). |
| D4 | Attacker forces excessive DHT writes from a peer | Veilid's own rate limiting at the transport level. Application-level retries use exponential backoff (1 s → 2 s → 4 s → … → 30 s). |
| D5 | Attacker keeps a community's gossip mesh saturated | Gossip TTL=5 caps fan-out depth; dedup cache prevents amplification; per-sender rate limit caps the worst single attacker. |
| D6 | DHT record garbage-collected during low activity | Mutual-aid record warming: idle peers `get_dht_value` subkey 0 of every community record every 5 minutes. Refreshes the network-side TTL without transferring payload. |
| D7 | Any peer crashes or exhausts a Rekindle node through Veilid's message parsing: a peer info with non-ASCII dial info or node ids, a safety-route hop with no next hop, an answer of a type we did not ask for, a DHT question with a bad record-key kind or length, a short FindNode id (remote panics); a peer info with oversized dial-info, relay or signature lists (hundreds of MB and a second of CPU per message) | Fixed in Veilid 0.5.8, which Rekindle pins (ADR 0017, plan VU): those inputs are refused instead of panicking, and the lists are capped before they are allocated or walked (Veilid `CHANGELOG.md:22-24`). The 0.5.8 node also stops serving its peer set to bootstrapping nodes (`serve_direct_bootstrap = false`, L2). |
| D8 | On a mixed-version network, a peer poisons the sequence numbers 0.5.7 storage nodes hold for a record we write (the DHT transaction schema-check bypass that 0.5.8 storage nodes close): a poisoned node refuses our legitimate sets and never counts toward consensus, and its inspect answers carry an inflated seq with no value to validate | **Residual until the public network upgrades.** An inspect seq is unauthenticated, so a reader treats "the network says newer, a forced get returns nothing newer" as a non-event: `RecordPool::read_changed` keeps its copy when the get finds nothing newer or fails (`rekindle-record-pool/src/read.rs`). Built (plan VU6): the pool remembers, per open record and subkey, a claimed inspect seq that the forced get did not confirm (`rekindle-record-pool/src/refetch.rs:49-113`, held in `lib.rs:143`); the same or a lower claim is never fetched again and a higher one at most once per backoff window, 60 s doubling to 3600 s on the pool's monotonic `tokio::time` clock (`refetch.rs:35-37`, `:57-61`, `:75-102`); a capped subkey is read locally, or not at all with no local copy (`read.rs:145-178`); any get at or above the claim clears it (`read.rs:182-200`); a close or restart forgets it, costing one fetch. `read_hinted` stays uncapped, since a value change obliges the network read. So a poisoned seq drives one fanout, not one per inspect (`refetch_tests.rs`). A set that a poisoned replica set holds below consensus answers `TryAgain` and reports `Offline`, never a false "stored". |

### E — Elevation of Privilege

| ID | Threat | Mitigation |
|----|--------|------------|
| E1 | Member without `MANAGE_CHANNELS` creates a channel | Reader-validates: the entry is in the writer's subkey but readers drop it. The writer's view is corrupted only locally. |
| E2 | Member without `BAN_MEMBERS` issues a ban | Same as E1. |
| E3 | Compromised admin issues mass bans / channel deletions | Limited mitigation: an admin with `BAN_MEMBERS` legitimately can ban anyone. Audit log records every governance change with author pseudonym and `lamport`. Other admins can `Unban` if they have higher role position. There is no "undo all" because that would itself be an `ADMINISTRATOR`-class action. |
| E4 | A non-rotator writes a `MEKGenerationBump` to claim authority | Reader-validates rotator (S5). |
| E5 | Daemon escalates from another local user's account | UCred binding in Noise IK prologue: the prologue includes both PID and UID. A different UID dialing the daemon's socket fails the handshake even if it's on the same machine. |
| E6 | A low-rank moderator, or one colluding with an attacker, re-admits a forked member at a rank above its own | ADR 0022 (Proposed; plan E3.2 S7d-keys): a `DeviceReadmit` counts from a `KICK_MEMBERS` holder who outranks the target's highest role (the kick rule), restoring only the roles at or below the co-signer's rank, or from one of the target member's own device keys that is effective at the readmit's deps and was endorsed before the member's first fork, at most one per checkpoint interval and shown to the member's devices and `KICK_MEMBERS` holders (corrected 2026-10-24, premise check: it read "counts only from", which dropped ADR 0022 rule 7's own-key path); its body binds the fork evidence and the incarnation, so one co-signature cannot answer a later fork. Nobody outranks a creator, so a creator with no pre-fork key left recovers only through a creator-signed `CommunitySuccessor` (ADR 0011, plan E3.2 S7s), which members are offered, never moved into; two conflicting pseudonym-signed successors void each other and neither is offered. |
| E7 | A member pre-endorses many device keys so that each can equivocate once, showing split views before it is caught | ADR 0022 (Proposed): at most 6 effective device keys per member in total, the first device included (Signal: the primary plus 5 linked; corrected 2026-10-24 from 5, prod research batch 7), enforced by readers, judged at each op's causal deps (never on the converged set, so a late endorsement cannot void accepted ops), and an endorsement whose signed extended set already holds 6 live keys is inert in every carrier; one fork per member, counted per device key (both copies of a cloned key retiring it are one fork): after a member's first fork (or any `DeviceRetired`), every new key needs a co-signed `DeviceReadmit`, except the one replacement carried in the same op as the member's first `DeviceRetired` (with two such carriers, only the lowest `(lamport, op_hash)` one's; corrected 2026-10-24, premise check), whose own-key co-signer must be a key endorsed before that first fork (a re-admitted key never co-signs, so approvals do not chain), at most one own-key readmit per member per checkpoint interval (plan E3.2 S7c; Signal's link bucket), each shown on all the member's devices and to `KICK_MEMBERS` holders (amended 2026-10-24, prod research batch 10); and a second fork (fork evidence for a second distinct key, pre-endorsed ones included; a `DeviceRetired` retires only its own key and is never a second fork) retires all of them, leaving only the moderator path (or a creator's `CommunitySuccessor`), so pre-endorsing keys buys at most one fork; the member is flagged to others. |

## 4. LINDDUN Privacy Threats

### L — Linkability

| ID | Threat | Mitigation |
|----|--------|------------|
| L1 | Linking the user across communities | Per-community pseudonym derivation; no shared identifier between communities. |
| L2 | Linking the user to their Veilid network identity | Veilid safety routes provide sender anonymity (3 hops, `ANONYMITY_HOP_FLOOR`). Veilid private routes provide receiver anonymity; their depth is the **inbound route depth**, set per route. **Target: 3 hops on both inbound route classes (General and Media)**, with the hop adjacent to us pinned so loss alone never redraws it (Tor vanguards). At 1 hop, the entry hop's PeerInfo (its IP) is in the clear in every route blob we hand out, and that hop connects directly to us, so any caller learns the one node that knows our IP (Veilid 0.5.8 fork `route_assemble.rs:201-211`; 0.5.7 `:199-216`, `rpc_route.rs:97`). **Today inbound routes are 1 hop** (`default_route_hop_count` = 1; `own_routes.rs` Media `hop_count: 0` = default); plan Phase RH sets `PrivateSpec.hop_count = 3` per route and leaves Veilid's pool default at 1. No app-triggered route replacement driven by measured loss (selective DoS would steer path choice; Tor Prop 271, vanguards-spec §1), and no full PeerInfo in the innermost hop. **Veilid 0.5.8 steers on loss internally, and that includes the hop next to us** (plan VU, register #10): a question that lost one to a destination demotes that safety route for that destination (`route_select.rs:321-410`, `route_stats.rs:462-473`; every unpinned question, never `app_message`), and a route that dies by loopback failures or lost-destination evidence puts **all** its hops, the entry hop included, on an avoid list for that ordering's new allocations until the hop or our peer info changes or the online state changes (`route_stats.rs:716-787`, `dead_route_hops_to_avoid` `:785`; applied in `route_allocate.rs:110-136`; CHANGELOG 0.5.8). An adversary who drops our traffic can therefore force the replacement of our 1-hop inbound route onto a different entry hop, where 0.5.7 drew at random. No Rekindle code triggers loss-driven replacement (`OwnRoutes` replaces only on `RouteChange` and `TryAgain`), so the conflict is with the pinned-hop target, not today's code: **plan RH1b's fork design must exempt the pinned hop from `dead_route_hops_to_avoid`** (ADR 0017 rule 2), or the pin would fight Veilid's avoidance. Our 1:1 calls are pinned to the `Sending` safety route (plan RH1r), and Veilid hands that same route to every callee as the call's reply route (symmetric routing; Veilid 0.5.8 fork `rpc_processor/destination.rs:485-525`, `route_compile.rs:436-447`; 0.5.7 `destination.rs:466-508`, `route_compile.rs:401-410`). **Residual (RH1r, pre-existing):** a pinned route that is released or marked for release when the call compiles is not used; Veilid falls back to a pool safety route for that call, silently. **Residual (VU5 finding, 2026-11-15):** a pinned reply route has no Veilid failover: loss-driven re-selection is skipped whenever a reply route is set (`route_compile.rs:566-568`, `:607-616`) and the preferred route skips loss demotion (`route_select.rs:148-180`), so a broken path on our `Sending` route drops questions until the route dies (5 loopback failures for an unordered route); Rekindle's own route liveness (plan RH1r-c) is the only detector. **Documented limit:** a callee learns our `Sending` route's blob, which is the same for every peer we call while the route lives. Two callees who compare blobs can therefore link their calls to one sender. The blob gives no more than an inbound route blob (its entry hop, never our IP at 3 hops), and the link holds until `OwnRoutes` replaces the route. Application-level pseudonyms further decouple. **Community route blobs (`M`/6, Z13):** a member's general route blob is on its member record's `M`/6, readable by anyone who holds the record key (past, kicked and banned members included); at 1 hop each blob names the entry hop next to us, so this audience can find that hop until Phase RH's 3-hop inbound routes land. |
| L3 | Linking presence updates to a stable identifier | Presence is published to the per-community member registry under the community pseudonym, not the master identity. |
| L4 | Linking a user's relay-routed message to their direct messages | Different Veilid private routes, different envelope formats, different timing. |

### I — Identifiability

| ID | Threat | Mitigation |
|----|--------|------------|
| I-priv1 | A non-friend identifies the user from public DHT records | Profile records are encrypted; only the owner key (held by the user) and friends with explicit access can decrypt the friend list / mailbox / presence. |
| I-priv2 | A community member identifies the user across communities | Pseudonym separation (L1). |
| I-priv3 | A network adversary identifies the user from packet timing | Veilid safety routes (sender side) and private routes (receiver side, L2) on every path. The hop count is a 3-hop Tor-class floor (`ANONYMITY_HOP_FLOOR`) that no class, setting or admin policy can lower (config may raise it) — voice and video get the same anonymity as text, never a lower-hop fast path, and there is no direct peer-to-peer media path, by default or per call. Today the floor is enforced on the safety side only; the private side is 1 hop until plan Phase RH (L2). |

### N — Non-repudiation (privacy-violating)

The flip side of R1/R2: pseudonym-level non-repudiation is *useful* for
moderation but creates a record that the user themselves cannot deny
*to other community members*. We accept this — community moderation
requires it. Across communities, pseudonym separation prevents the
non-repudiation from leaking globally.

### D — Detectability

| ID | Threat | Mitigation |
|----|--------|------------|
| D-priv1 | An observer detects that the user is online | Direct gossip and presence updates make this visible to friends/community members by design. To non-members, route metadata leaks "some Veilid traffic" but not "Rekindle traffic" specifically. |
| D-priv2 | An observer detects that a community exists | Communities are addressed by DHT key. Without the key, there is no global directory to enumerate. Discovery is opt-in. |
| D-priv3 | Push-relay platform vendor detects "this device is using Rekindle" | Yes — any push notification reveals app installation. Mitigation: opt-in; users with this concern run Tier 2 only. |
| D-priv4 | A contact learns when the user read a message (attention, online times; an abuser saying "I know you read this", Freed et al. CHI 2018) | **Policy (plan E2.5):** DM read receipts are opt-in, reciprocal and one global setting, **off by default**. A receipt is a read-up-to marker with no timestamp, coalesced per peer and sent after a uniform random 2–10 s delay, only after the chat window is focused and the message rendered; never to pending, blocked or non-friend contacts and never while Invisible; the setting itself never goes on the wire, so "off" and "unread" look the same; received markers are ignored while our own setting is off. **No community or channel read receipts**: unread state stays local (and syncs only between the user's own devices). **Today:** no read receipts are sent; `is_read` is local only. Residual risk under A8: a coerced user can be made to turn receipts on. |
| D-priv5 | Delivery receipts reveal that a device is online, how many devices exist, or let a stranger probe activity (Careless Whisper, arXiv 2411.11194) | **Policy (plan E2.5):** `DmAck` is a transport ack, not a setting: sent only to accepted friends, only for user content that is durably committed, coalesced per peer with the same 2–10 s random delay, held while Invisible, and sent by one device per identity. Communities get no per-reader delivery receipts. **Today:** there is no peer delivery receipt; the desktop emits a local `DirectMessageAcknowledged` right after a send, even a failed one (`chat_runtime.rs`), which E2.5 deletes. |

### D — Disclosure (data)

Covered under STRIDE I.

The send log (`message_send_log`, plan E2.5, D24) keeps the plaintext of each user-content 1:1 message we sent, at rest under the identity database's SQLCipher key, for `DELIVERY_HORIZON` (30 days) so a peer can ask for it again; a resend needs the ratchet key we logged, and rows past the horizon are pruned at unlock.

The DM chains (plan E2.5, D23) put each direction of a conversation on the DHT: the sender's dm participant record (owned by a random keypair only the sender holds, one per conversation, unlinked to their identity key on the network) points at a backward-linked list of sealed single-subkey pages, each written once and then owner-less. A page holds only `'D' ‖ DmFrame` ratchet frames, as sent by `app_message`: no plaintext, no identity key in the clear, but the DHT nodes storing it, and anyone given the participant record key (only the peer, inside the ratchet, as `StreamRecord`), learn each page's size, the number of entries, and when each page and the `HEAD` subkey were written, so the record reveals the conversation's volume and timing in one direction, and its write cadence links the pages of one record. Entries stay readable until the sender retires them: the retire mark in `HEAD` moves past an entry once the peer acknowledged it (no `dm` outbox row left) and it is older than `DELIVERY_HORIZON` (30 days, ruling Q2b), after which readers stop fetching it; the retired pages themselves stay on the DHT until Veilid's record expiry drops them, since a sealed page has no owner to delete it. A reader still behind the mark is shown a stored `HistoryGap` marker, never a silent loss. Under A8 (a coerced device), the chain gives no more than the device's own history does; to the DHT (A2/A3), it is the same ciphertext the network already carried, kept up to 30 days past acknowledgement.

### U — Unawareness

| ID | Threat | Mitigation |
|----|--------|------------|
| U1 | User does not realise their game-detect data is shared | Opt-in per identity; off by default until the user enables it. |
| U2 | User does not realise their presence is visible to community members | UI shows the presence status next to the user's avatar in every community window. Presence is a fundamental feature, not hidden. |
| U3 | User does not realise their banner / nickname / avatar is visible community-wide | Profile editing UI is the same place the values are displayed. |
| U4 | User does not realise a paired device retains the master secret after unpair | Documented limitation. Master-secret rotation is an open problem (see [sync.md](../architecture/sync.md) "Open work"). |

### N — Non-compliance

We are pre-1.0; no GDPR / HIPAA / regulatory promises are made. The
[`../../SECURITY.md`](../../SECURITY.md) policy describes the
disclosure process for user-impacting issues regardless of regulatory
status.

## 5. What we deliberately accept

These are conscious tradeoffs documented for full transparency:

- **Call latency from anonymity routing (L2, I-priv3).** With 3 hops on each side, projected
  mouth-to-ear is ~470–500 ms median. The ship gate is G.114 §4's own measure, mean one-way *route*
  delay ≤ 400 ms; mouth-to-ear ≤ 400 ms and E-model R ≥ 70 (G.107 Default class) are measured
  targets, and the call bar shows the live G.109 category, naming anonymity routing as the cause
  when quality drops. No faster, lower-hop path is offered.
- **Call-media leaver window (I13).** A leaver may hear up to ~10 s after leaving under rapid churn.
- **A chosen login skin is visible before unlock (Z16, I15).** Anyone at the screen, or a seizer of
  the locked device (A7), sees the login skin the user chose, and a coercer (A8) can make the user
  set one. Only an explicit, reviewed choice puts a non-built-in skin there, stripped of everything
  that names it.

- **Pre-join governance reads by invite (I16).** Whoever holds a live
  invite's code can read the community's current governance state before
  joining, as a joiner can; the code already lets it join. Seeing the
  `InviteCreated` (its public `code_hash`) is not enough. The same holds
  for joining (plan E3.2 S10): `MemberJoined.invite_proof` is signed by
  the invite's join key, derived from the code, with `code_hash` its public
  half, so a reader of the op log (a kicked member, an I16 outsider) cannot
  make one, nor replay a seen one for another pseudonym
  ([`crypto-primitives.md`](crypto-primitives.md) §7).
- **Pre-ban content disclosure (I4).** A banned member retains content
  they cached before the ban. This is intrinsic to E2E P2P — once
  bytes leave the sender, no protocol can revoke them.
- **Memory-resident secret exposure (I7).** A live memory dump on an
  unlocked, running app reveals active secrets. Zeroize-on-drop limits
  the exposure window but cannot eliminate it.
- **Pseudonym non-repudiation within a community (R1, N).** Required
  for moderation; bounded by community boundaries via pseudonym
  separation.
- **Push-relay metadata leakage (I9).** Opt-in; self-hostable.
- **Game-detection app installation visibility (D-priv3).** Inherent
  to push notifications.
- **Mass-spam window during gossip propagation (D1).** A spammer can
  burst ~5 messages before the rate limit + ban catches up. Honest
  clients retroactively filter via CRDT merge.
- **DoS from a malicious admin (E3).** A legitimate admin abusing
  their permission can damage the community. Audit log + role
  hierarchy + ban-by-other-admin partially mitigate. There is no
  "platform-level intervention" because there is no platform.

## 5b. Frontend WebView attack surface

The Tauri shell renders SolidJS code in the system WebView (WebKit on
macOS, WebView2 on Windows, WebKitGTK on Linux). That is a real
browser engine: full JavaScript, full DOM, full CSP semantics, full
inheritance of any vulnerabilities the engine itself has. This section
enumerates the web-app attack classes that genuinely apply, even
though Rekindle exposes no public HTTP server.

### W1 — Stored XSS via peer content

Any field rendered from another peer is attacker-controlled until
proven otherwise: friend display names, profile bios, custom-status
strings, presence game info, community names, channel names, role
names, message bodies, link-preview titles/descriptions, custom
emoji names, embed fields, governance entry metadata.

| Mitigation | Status |
|------------|--------|
| SolidJS escapes `{value}` interpolation by default | **Met** |
| `innerHTML` allowed only for SVG generated by trusted Rust IPC | **Met** — one site (`AddDeviceModal.tsx:296`), audited |
| Markdown / link-preview / custom-emoji rendering uses DOMPurify | **Open** — features pending; gate documented in [`frontend-rendering.md`](frontend-rendering.md) |
| Semgrep `rekindle-no-inner-html` rule | **Met** — `.semgrep.yml` |
| Playwright XSS injection suite | **Met** — `e2e/security/xss.spec.ts` |

### W2 — Hostile OS deep links

Any web page or app can open a `rekindle://` URL: a community invite
(`rekindle://invite/{governance key}/{secrets key}/{32-hex code}`), a
friend invite (`rekindle://<base64url signed blob>`) or a pairing URI.
A link must never act without the user's consent, never reach the DOM
raw, and never reveal its secrets to the webview.

| Mitigation | Status |
|------------|--------|
| Strict Rust parsing: `rekindle_types::invite::{InviteLink, DeepLink}` (exact shape, ≤ 256 B invites, ≤ 16 KiB friend invites); hostile corpus unit-tested there | **Met** |
| Friend invites decoded, signature- and recency-checked locally before anything is shown | **Met** — `src-tauri/src/deep_links.rs` |
| Consent before any network access: the backend holds the link; the buddy list shows a 64-bit key fingerprint and acts only on Confirm | **Met** — `DeepLinkConsentDialog.tsx`; `e2e/security/deep-link.spec.ts` |
| Pairing URIs refused from OS deep links (pasted or scanned in Settings → Devices only) | **Met** |
| Pending link cleared on logout, so it is never offered to the next user | **Met** |
| Semgrep `rekindle-deep-link-no-direct-render` rule | **Met** |

### W3 — CSP bypass via inline scripts / eval

A loose CSP turns every other XSS mitigation into theatre. The
declared CSP in `src-tauri/tauri.conf.json` `app.security.csp` is the
load-bearing defence.

| Directive | Value | Reasoning |
|-----------|-------|-----------|
| `default-src` | `'self'` | Whitelist baseline. |
| `script-src` | `'self'` | No `'unsafe-inline'`, no `'unsafe-eval'`. |
| `style-src` | `'self' 'unsafe-inline'` | Tailwind 4 + SolidJS need it. Trade-off documented. |
| `img-src` | `'self' data:` | `data:` serves avatars, backend-built expression images and the QR SVG (W1). No remote origin: a peer-chosen image URL would leak every reader's IP. Enforced by `tests/csp_policy.rs`. |
| `media-src` | `'self' blob: data:` | Voice messages play from `blob:` URLs of backend-fetched bytes; soundboard clips from backend-built `data:` URLs (media type sniffed against an allowlist). |
| `connect-src` | `ipc: http://ipc.localhost` | Locks outbound `fetch` to the Tauri IPC bridge. |
| `font-src` | `'self' data:` | Standard. |
| `object-src` | `'none'` | Forbids `<object>`/`<embed>`/`<applet>`. |
| `frame-ancestors` | `'none'` | No third party can frame our window. |
| `frame-src` | `'none'` | We never embed iframes. |
| `base-uri` | `'self'` | Defeats `<base>` injection redirecting relative URLs. |
| `form-action` | `'self'` | Defeats form-action hijack to attacker-controlled origin. |
| `manifest-src` | `'none'` | We don't ship a web app manifest. |
| `worker-src` | `'self'` | Service / Web Workers, if any future code uses them. |

| Mitigation | Status |
|------------|--------|
| CSP enforcement verified at runtime | **Met** — `e2e/security/csp.spec.ts` |
| Semgrep `rekindle-no-eval`, `rekindle-no-inner-html` | **Met** |

### W4 — Capability privilege escalation via Tauri ACL

The Tauri capability files (`src-tauri/capabilities/*.json`) control
which commands each WebView can call. A loose ACL (broad `*:default`
permission bundles, webview creation) lets a renderer-side XSS escalate
into invoking sensitive Rust commands or spawning a window under a
privileged label.

| Mitigation | Status |
|------------|--------|
| App-command ACL manifest exists | **Met** — `build.rs` derives the command list from `src/invoke.rs`, so `tauri-build` emits one `allow-*`/`deny-*` per command and Tauri enforces the per-window grants |
| Per-window allow-list rather than global grants | **Met** — one `app-*.json` per window family; `app-login.json` is pinned to the 7 pre-auth commands |
| `default.json` limited to window chrome | **Met** — only the `core:window:*` actions the titlebar/drag region use; no `core:event` (events arrive on each window's own channel, ADR 0013) |
| No webview creation from the frontend (S8) | **Met** — `core:webview:*` not granted; windows are created only by Rust in `src/windows/` |
| No `core:default` / plugin `*:default` bundles | **Met** — no plugin grants in any window. Dialogs and URL opening run in Rust behind `upload_attachment`, `download_attachment` and `open_external_url`; `tauri-plugin-notification` is gone (OS notifications from Rust, `os_notify.rs`) |
| Debug commands absent from release | **Met** — `#[cfg(debug_assertions)]` on the commands; their grants live in `capabilities-dev/`, loaded only in debug builds |
| Policy enforced in CI | **Met** — `src-tauri/tests/capability_policy.rs` (no blanket grants, login allowlist, every command granted, window patterns match `windows.rs`, debug grants only in `capabilities-dev/`) |
| Description field documents rationale | **Met** — each capability file's `description` field |
| New permissions require security review | **Process** — PR template flags any change touching `capabilities/` |

### W5 — Prototype pollution

Merging `JSON.parse` of attacker-controlled data into application
state can poison `Object.prototype`. Tauri's IPC bridge already
serialises with structured types (Cap'n Proto on the Rust side), but
any `JSON.parse` of data coming from a peer (link-preview body,
external embed) is a vector.

| Mitigation | Status |
|------------|--------|
| Cap'n Proto decode for all peer-content envelopes | **Met** |
| `app.security.freezePrototype: true` — `Object.prototype` frozen before any app script | **Met** — `tests/csp_policy.rs` |
| Semgrep `rekindle-no-prototype-pollution-merge` rule | **Met** |
| `Object.create(null)` for any merge target with peer-controlled keys | **Convention** — enforced by review |

### W6 — Open redirect / location injection

Direct assignment to `window.location` from variable input lets
attackers redirect users to phishing pages.

| Mitigation | Status |
|------------|--------|
| Every window's navigation is limited to the app origin (`on_navigation`), and `window.open` / `target=_blank` are denied (`on_new_window`) | **Met** — `src-tauri/src/windows/navigation.rs` |
| External links open only through the backend `open_external_url`: https-only, ≤ 2048 B, after a native confirm naming the host (punycode for IDNs); the opener plugin's JS link interception is off and no window holds an opener grant | **Met** |
| Semgrep `rekindle-no-unchecked-href-assignment` rule | **Met** |

### W7 — WebView CVEs

The WebView engine is an inherited dependency. CVEs in WebKit /
WebView2 / WebKitGTK / libsoup affect Rekindle directly.

| Mitigation | Status |
|------------|--------|
| Weekly query of GitHub Advisory DB for WebView-engine CVEs | **Met** — `.github/workflows/webview-cve-check.yml` |
| Auto-issue on high/critical advisory match | **Met** |
| `docs/user/install.md` documents minimum platform versions | **Met** |
| Triage SLA matches direct-dependency CVE policy | **Met** — `incident-response.md` |

### W8 — Secret leakage via `console.log`

Browser devtools and (potentially) external observability sinks pick
up `console.log` output. Logging a key, MEK, or passphrase exposes
it.

| Mitigation | Status |
|------------|--------|
| Rust-side `Debug` impls redact sensitive fields | **Met** |
| Log files carry no identifiers (S16): every line from every target, including `veilid_core`/`veilid_api`, passes through `rekindle_utils::log_scrub` at the sink — Veilid keys, record keys, hex identity/pseudonym/channel/call ids, UUIDs and IP addresses become per-process keyed-hash tags (Signal-Android `Scrubber` design); panic output is scrubbed the same way | **Met** — tested against real veilid-core `Display`/`Debug` output (`rekindle-protocol/tests/log_scrub_veilid.rs`) |
| Frontend convention: never `console.log` secret-bearing fields | **Convention** |
| Semgrep `rekindle-no-secret-in-log` rule | **Met** |
| Biome `noConsole` rule (warns on `console.log`, allows `warn` / `error`) | **Met** |

### W9 — Frontend cryptography drift

If JS-side code performs cryptographic operations independent of
Rust, the Rust `rekindle-secrets` Tier-2 sole-crypto-boundary
property breaks. A subtle-bug in the JS impl wouldn't be caught by
Rust audits.

| Mitigation | Status |
|------------|--------|
| All crypto via Rust IPC commands | **Met** |
| Semgrep `rekindle-no-frontend-crypto-primitives` rule | **Met** — bans `crypto.subtle.*` in `src/**` |

### Documents

- [`frontend-rendering.md`](frontend-rendering.md) — DOMPurify gate for markdown / link previews / custom emoji
- [`crypto-primitives.md`](crypto-primitives.md) — primitive selection (covers W9)
- `e2e/security/` — runtime tests for W1, W2, W3
- `.semgrep.yml` — SAST rules covering W1, W2, W5, W6, W8, W9
- `.github/workflows/webview-cve-check.yml` — covers W7

## 6. Open issues and deferred mitigations

Tracked in [`../roadmap.md`](../roadmap.md) and the issues listed in
[`../../SECURITY.md`](../../SECURITY.md):

- **Master-secret rotation.** Today, a paired device that unpairs
  retains the master secret. Rotation is an open design problem
  because it invalidates pseudonyms across communities.
- **Reproducible builds.** Documentation and CI verification deferred
  until first tagged release.
- **Out-of-band verification UI.** TOFU works, but the explicit
  verification flow (numeric safety codes / QR comparison) needs
  polish before users can rely on it.
- **Gossip metadata analysis.** A network adversary observing many
  Veilid relays can do traffic-pattern analysis. Veilid's mitigations
  (safety and private routes) help but do not eliminate this. Rekindle fixes the
  hop count at a 3-hop Tor-class floor on every path, on the safety **and**
  the private side — including voice and video — rather than exposing a lower-hop fast path, accepting the
  latency cost as a deliberate privacy tradeoff.
- **Inbound route depth.** Receiver-side private routes are 1 hop today;
  3 hops with the adjacent hop pinned is plan Phase RH (L2). Pinning the
  adjacent hop needs a Veilid change (`AllocateRouteParams` has only
  `avoid_nodes`), which goes through the fork policy of ADR 0017. Since
  Veilid 0.5.8 that change must also exempt the pinned hop from Veilid's
  dead-hop avoidance (`dead_route_hops_to_avoid`), which otherwise redraws
  the entry hop after a loss-killed route (L2).
- **Call media keys.** The joiner and device-seizure properties in I13
  hold only after plan step C7.29.

## 7. How to report a finding

See [`../../SECURITY.md`](../../SECURITY.md). The disclosure channel
is GitHub's private vulnerability flow plus an email backup. We commit
to acknowledging within 3 business days and substantive triage within
10. Findings that align with documented limits in §5 are still welcome
— we may have under-estimated the risk.
