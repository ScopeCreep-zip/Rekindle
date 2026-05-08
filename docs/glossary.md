# Glossary

Project-specific vocabulary used across Rekindle's codebase and
documentation. Sorted alphabetically. Terms inherited from referenced
external systems (Veilid, Signal, Death Stranding) are included where
they have a specific Rekindle meaning.

---

**ADR (Architectural Decision Record).** A short, append-only
document recording why an architecturally significant choice was
made, what alternatives were considered, and what consequences
follow. Lives in [`decisions/`](decisions/). Format follows
[MADR 4.0](https://adr.github.io/madr/).

**AppCall (Veilid `app_call`).** Acknowledged request-response
datagram primitive. The caller sends bytes; the callee receives,
processes, and replies. Used in Rekindle for MEK delivery, file chunk
transfer, bootstrap bundles, voice signaling, friend-request
handshakes — anywhere we need confirmation of receipt. Reserved for
operations that genuinely need a response, because each `app_call`
holds a pending connection slot on both sides and can saturate
Veilid's connection table if abused. See
[`architecture/communities.md` §2.6](architecture/communities.md).

**AppMessage (Veilid `app_message`).** Fire-and-forget datagram
primitive. Maximum payload ~32 KB. Used in Rekindle for gossip,
voice frames, presence updates, ephemeral notifications. The bytes
are uninterpreted by Veilid — Rekindle layers its own envelope
format and signatures on top. Voice uses
`SafetySelection::Unsafe(Sequencing::NoPreference)` for low latency;
chat uses safety routes for sender anonymity.

**Argon2id.** Memory-hard passphrase KDF used to derive the
Stronghold vault key from the user's passphrase. Standardised in
[RFC 9106](https://datatracker.ietf.org/doc/html/rfc9106). See
[`security/crypto-primitives.md` §9](security/crypto-primitives.md).

**AttachmentBitmap.** A peer's per-attachment list of which chunks
they currently hold, broadcast via gossip so requesters can do
swarm fetch. See
[`architecture/files.md`](architecture/files.md).

**AttachmentOffer.** The wire-level announcement that an attachment
exists, containing its `attachment_id`, filename, MIME type,
chunk hashes, Merkle root, and the FEK wrapped under the channel
MEK. Lives at `crates/rekindle-types/src/attachment.rs`.

**Bitfield permissions.** 64-bit `u64` representing a member's
permissions in a community, modelled on Discord's permission
bitfield. See
[`architecture/communities.md` §9](architecture/communities.md).

**Bootstrap pointer.** Optional immutable DFLT record (Record 0 of a
community) that points at the governance and registry record keys.
Used for discovery only — not for governance. See
[`architecture/communities.md` §3](architecture/communities.md).

**BootstrapBundle.** A snapshot delivered via `app_call` from an
inviting member to a joiner: pre-merged governance entries, online
member list with route blobs, channel MEKs wrapped per-recipient,
last 50 messages per channel, owner keypair wrapped for the
joiner. Reduces a 30-DHT-read join sequence to a 1-RTT operation.

**BLAKE3.** Modern, fast, keyed cryptographic hash. Used in
Rekindle for deterministic rotator selection
(`blake3(departed ‖ self)` lowest-hash wins) and content
addressing. Distinct from SHA-256 to give defense in depth across
hash-function code paths. See
[`security/crypto-primitives.md` §7](security/crypto-primitives.md).

**Capabilities (Tauri).** Permission file at
`src-tauri/capabilities/default.json` controlling which Tauri APIs
the WebView can call. Rekindle declares only the capabilities
actually used by the frontend.

**Cap'n Proto.** The wire serialisation format for messages and DHT
record contents. Schemas live at `schemas/`. The generated Rust
modules are placed at the consuming crate's root (e.g. `pub mod
foo_capnp { include!(...) }` in `lib.rs`).

**Cargo (Lost Cargo).** Rekindle's chunked, peer-cached file
delivery system. The name comes from Death Stranding —
heavy *cargo* sits at depots, and porters who happen to be
carrying it deliver to anyone who asks. See
[`architecture/files.md`](architecture/files.md).

**Cascade fallback (MEK rotation).** If the deterministically-
selected MEK rotator is offline, the next-in-line rotator takes
over after a 30-second window, listing the failed rotator in
`cascade_skipped`. Readers verify each skipped member was actually
offline before accepting the cascade. See
[`architecture/communities.md` §5](architecture/communities.md).

**Channel record.** A SMPL DHT record per channel (`o_cnt: 0`, 255
member subkeys) where each member writes `ChannelEntry` variants
to their own subkey. Records 3+ in a community.

**Chiral Network.** Death Stranding-inspired mental model for
Rekindle's communities: every node is a porter, every node is a
waystation, mutual aid is the only infrastructure. Captured
architecturally as flat SMPL governance + three-path delivery +
mutual-aid patterns. See
[`architecture/communities.md` §1](architecture/communities.md).

**Chiral notification model.** The principle that gossip carries
*notifications* (metadata: channel, subkey, sequence, content
hash), not *cargo* (ciphertext). Ciphertext lives in the SMPL DHT
record; notifications travel the mesh. Critical privacy property
for harvest-now-decrypt-later defence.

**ChunkCache.** Filesystem-backed LRU cache holding decrypted file
chunks. Per-community. Eviction skips pinned attachments. Default
budget 1 GiB. See
[`architecture/files.md`](architecture/files.md).

**Communities v1.0 / v2.0.** v1.0 was the original
rotating-coordinator design that has been removed. v2.0 is the
flat-SMPL-governance "chiral network" architecture in current use.
See [ADR 0003](decisions/0003-flat-smpl-governance.md).

**Continuation chain.** When a channel SMPL subkey approaches
~28 KB, the writing member writes a `continuation_record_key`
pointer and starts a new SMPL channel record. Readers follow the
chain. The "timefall" pattern in Death Stranding terms — old
records age into cold storage.

**CRDT (Conflict-free Replicated Data Type).** A data structure
whose merge operation is associative, commutative, and idempotent,
guaranteeing convergence across replicas regardless of update
order. Rekindle's governance state is a CRDT computed by
`rekindle-governance::merge` over all community member subkeys.

**Dedup cache.** 1024-entry FIFO ring buffer keyed by
`(sender_pseudonym, lamport_clock)` that drops duplicate gossip
envelopes. Rate-limits epidemic broadcast amplification.

**DFLT record.** Single-owner Veilid DHT record schema. Only the
creation keypair can write any subkey. Used in Rekindle for
profile records, the optional bootstrap pointer, and the personal
cross-device sync record. Distinct from SMPL.

**DHT (Distributed Hash Table).** Veilid's distributed key-value
store. Records are addressed by `TypedKey` (crypto kind + public
key). Each record has up to 65 535 subkeys, each up to 32 KiB.
Replication factor 5 (1 primary + 4 neighbors).

**DHTLog.** Append-only ring-buffer pattern over DHT subkeys,
referenced occasionally in design discussions. Rekindle does not
use DHTLog as a primary primitive; we use SMPL records with
explicit subkey ownership instead.

**Diátaxis.** Documentation framework distinguishing tutorials /
how-to / reference / explanation. Rekindle's `docs/user/` follows
this structure for user-facing material.

**Direct call.** 1:1 voice or video call between two friends,
distinct from community voice channels. Uses a per-call key
derived via X25519 ECDH plus HKDF-SHA256, scoped by `call_id` so
parallel calls between the same pair produce different keys. See
`crates/rekindle-calls/`.

**DM / Group DM.** Direct messages between 2 (DM) or 3–8 (group DM)
peers. SMPL record with `o_cnt: 0`. DM MEK is derived
deterministically via X25519 ECDH between identity keys; group DM
MEK is randomly generated and wrapped per-participant. See
[`architecture/communities.md` §11](architecture/communities.md).

**Double Ratchet.** Signal Protocol's per-message symmetric ratchet
combined with a Diffie-Hellman ratchet for forward and backward
secrecy. Used in Rekindle for 1:1 friend messaging.

**Ed25519.** Edwards-curve digital signature algorithm. Identity
keys, per-community pseudonyms, gossip envelope signatures, and
governance entry signatures. Standardised in
[RFC 8032](https://datatracker.ietf.org/doc/html/rfc8032).

**EventCategory / SubscriptionFilter.** IPC bus subscription types
on the daemon track. Clients subscribe to event categories
(messages, presence, voice, governance, …); the daemon's event
router fans events out to matching clients.

**Fan-out degree (D).** The number of gossip targets a sender
relays to: 1–20 members → min(N-1, 6); 21–60 → 6; 61+ → 8;
plate-gated → 8 per segment + 4 cross-segment. See
[`architecture/communities.md` §3](architecture/communities.md).

**FEK (File Encryption Key).** Per-file 32-byte AES-256 key used
to encrypt every chunk of an attachment. The FEK is wrapped under
the channel MEK in `AttachmentOffer`. Decoupling chunk encryption
from MEK rotation lets cached chunks survive MEK rotations
unchanged. See [`architecture/files.md`](architecture/files.md).

**Forward secrecy.** A property of an encryption scheme where past
session keys cannot be derived from a current key compromise. In
Rekindle: Signal's Double Ratchet for 1:1, MEK rotation for
communities, ECDH ratchet for DM, per-call key for direct calls.

**Frameless window.** Window with `decorations: false` and
`transparent: true` — no OS title bar, transparent edges. Every
Rekindle window is frameless to support the custom Xfire skin.
See [`architecture/ui-skin.md`](architecture/ui-skin.md).

**Genesis (entry).** The first write to a SMPL subkey (Veilid
sequence number 1). Always accepted regardless of permission
checks, because the community has no prior permission state to
validate against.

**Gossip mesh.** Epidemic-broadcast network laid over Veilid
`app_message`. Each sender picks D random online peers; each
receiver dedups, decrements TTL, processes, and re-broadcasts. See
[`architecture/communities.md` §3](architecture/communities.md).

**GovernanceEntry.** A typed entry written to a member's governance
subkey: `ChannelCreated`, `RoleDefinition`, `BanEntry`,
`MEKGenerationBump`, `SegmentAdded`, etc. The CRDT merges all
entries from all members into a `GovernanceState`.

**Hand-raise.** A `ChannelEntry::HandRaise { raised: bool }` written
in stage channels, indicating the audience member wants to speak.
Moderators promote raised hands by writing a role assignment for a
"Speaker" role.

**HKDF (HMAC-based Key Derivation Function).** RFC 5869 KDF used
throughout Rekindle: per-community pseudonym derivation, slot
keypair derivation, DM MEK derivation, DM ratchet step. Domain-
separated by an explicit `info` string per use. See
[`security/crypto-primitives.md` §8](security/crypto-primitives.md).

**InviteSecrets.** The decrypted payload of an invite link
containing the governance and registry record keys, the
`slot_seed`, channel keys, current MEK per channel, the inviter's
pseudonym + route blob. See
[`architecture/communities.md` §6](architecture/communities.md).

**IpcRequest / IpcResponse.** Wire-level enums on the daemon track
for the encrypted IPC bus between `rekindle-node` and clients
(`rekindle-cli`, future others). One variant per supported
operation. The daemon's match is exhaustive.

**Jitter buffer.** Adaptive buffer in the voice pipeline that
reorders out-of-order packets and absorbs network jitter. Sized
40 / 80 / 120 ms based on group size — see
[`architecture/voice.md`](architecture/voice.md).

**Lamport clock.** Per-sender logical clock incremented on every
gossip send / governance write. Used as the primary ordering key
in CRDT merges; ties are broken by lexicographic pseudonym.

**LINDDUN.** Privacy-threat taxonomy: Linkability, Identifiability,
Non-repudiation, Detectability, Disclosure, Unawareness, Non-
compliance. Used alongside STRIDE in
[`security/threat-model.md`](security/threat-model.md).

**Lost Cargo.** See *Cargo*.

**LWW (Last-Writer-Wins).** CRDT merge strategy where the entry
with the highest `lamport` (ties broken by pseudonym) takes
effect. Used for role definitions, community metadata, permission
overwrites, automod rules, and many more entry types. See
[`architecture/communities.md` §4](architecture/communities.md).

**Mailbox.** Per-friend DHT record for offline message delivery.
When direct routing is unreachable, messages are written here and
the recipient pulls them on next online cycle.

**MADR.** [Markdown Architectural Decision Records](https://adr.github.io/madr/),
the format used for ADRs in [`decisions/`](decisions/).

**MEK (Media Encryption Key).** AES-256-GCM key for community
channel content. One MEK per channel, with a monotonically
increasing generation counter. Rotates on member departure
(forward secrecy) via the deterministic rotator protocol. See
[`architecture/communities.md` §5](architecture/communities.md).

**MEKGenerationBump.** Governance entry written by the
deterministic rotator after MEK rotation. Carries
`trigger_departed`, `cascade_skipped`, and the new generation
number. Reader-validates rotator authority.

**Merge.** The pure CRDT function in `rekindle-governance` that
takes all subkey contents and produces a `GovernanceState`.
Deterministic, commutative, idempotent. Property-tested.

**Merkle root.** v1 file format: `SHA256(chunk_hashes
concatenated)`. v2 will use a true binary Merkle tree
([BEP-52](https://www.bittorrent.org/beps/bep_0052.html)) for
files larger than 28 MB.

**MCU (Multipoint Control Unit).** Audio mixer pattern. In
Rekindle, the *mutual-aid SFU* is the deterministic-elected peer
that fans out voice frames for >4-participant calls — see *SFU*.

**Mutual aid.** The chiral-network ethos: members contribute
bandwidth, storage, and relay capacity to their communities
because they benefit from the community's existence. Realised as
record warming, history advertisements, watch relay, bootstrap
bundles, gossip topology optimisation, and MEK relay. See
[`architecture/communities.md` §8](architecture/communities.md).

**Noise IK.** The Noise Protocol Framework pattern used by
`rekindle-node` for the encrypted IPC bus.
`Noise_IK_25519_ChaChaPoly_BLAKE2s` — initiator's static key
transmitted, responder's static key pre-known, X25519 DH,
ChaCha20-Poly1305 AEAD, BLAKE2s hash. UCred mixed into the
prologue for OS-level binding.

**`o_cnt`.** Veilid SMPL schema parameter: number of subkeys
reserved for the record's owner. Rekindle uses `o_cnt: 0` for all
community records — **no owner subkeys**. The owner keypair
remains as the record's address but cannot write to any subkey.

**Opus.** Royalty-free IETF-standard speech codec used by the
voice pipeline. Configured for VoIP mode at 48 kHz mono, 20 ms
frames, 32 kbps, in-band FEC. See
[`architecture/voice.md`](architecture/voice.md).

**OR-Set.** Observed-Remove Set — CRDT merge strategy where an
entity exists in the merged state if it has more "create" than
"remove" entries (matched by ID). Used for channels, categories,
threads, events.

**Pairing.** The cross-device handshake that transfers the master
secret + personal sync record key from an existing device to a
new one. One-time code + salt + record key (typically delivered
via QR), `app_call` with `PairingPayload`, `PairingAccept` reply.
See [`architecture/sync.md`](architecture/sync.md).

**Personal sync record.** A DFLT record per identity with 4
well-known subkeys (manifest / read state / preferences / device
list) used for cross-device synchronisation.

**Plate Gate.** Fractal-segmentation scheme that lets communities
exceed Veilid's practical 255-member-per-SMPL-record limit. Each
segment is its own join-semilattice; the community state is the
product CRDT under coordinate-wise join. See
[`architecture/communities.md` §7](architecture/communities.md).

**PreKey bundle.** Signal Protocol public-key material published in
advance to the user's DHT profile so peers can initiate sessions
asynchronously. Includes identity key, signed prekey, and one-time
prekeys.

**Presence.** A member's published status: online / away / busy /
offline plus optional custom message, voice channel, route blob,
game info, avatar reference. Lives in the per-community member
registry record. Refreshed every 15 seconds; doubles as a
liveness signal.

**Private route.** Veilid primitive for receiver anonymity. The
receiver creates the route and publishes the route blob; senders
import the blob and target it. The publisher's network identity
is not revealed. Routes expire after ~120 s of inactivity.

**Pseudonym.** A community-specific Ed25519 keypair derived via
HKDF(`master_secret`, `community_id`). Different communities
yield cryptographically unrelated pseudonyms. Provides
unlinkability of the user across communities.

**Q-pid equation.** The architectural slogan for "one equation
applied uniformly at every level": every multi-writer DHT record
in a v2.0 community uses the same SMPL schema with `o_cnt: 0`
and 255 member slots. See
[`architecture/communities.md` §3](architecture/communities.md).

**Reader-validates.** The principle that every reader of governance
state independently checks every entry's permission against the
CRDT-merged state. Invalid entries are silently dropped. There is
no privileged write path. The complement of *writer-enforced*
permissions in centralised systems.

**Record warming.** Mutual-aid pattern: idle peers periodically
`get_value` subkey 0 of every community DHT record to refresh
the network-side TTL. Keeps records alive in the Veilid DHT
during low-activity periods without transferring payload.

**RelayEnvelope.** Wire format for Strand Relay: a wrapper
containing `target_route` and `inner_payload`, sent through a
mutual friend who cannot read the inner payload (encrypted to the
recipient's key). See
[`protocol/relay.md`](protocol/relay.md).

**Rekindle blue.** The Xfire accent colour `#177cc1` used for
selected items, focus rings, links, and the in-game status
indicator.

**Rotator (MEK rotator).** The deterministically-selected member
who writes the new MEK after a member departure. Selected by
`argmin(blake3(departed_pseudonym ‖ self_pseudonym))`. Same
inputs everywhere → same output → no election. Cascade fallback
if the chosen rotator is offline.

**Schwarzschild principle.** The metaphor for what happens to a
community's creation keypair: the creation event collapses behind
a horizon, leaving only the structure (the DHT records, schemas,
subkey allocations) but not the authority that created them. The
public key is an address; the private key is shared as
infrastructure to all members; neither carries governance
authority. See
[`architecture/communities.md` §1](architecture/communities.md).

**SFU (Selective Forwarding Unit).** Pattern where one peer fans
out audio/video frames to listeners without transcoding. In
Rekindle, the **mutual-aid SFU** is the deterministic-elected
online voice participant for calls with more than 4 members.

**SignedEnvelope.** The signed wire format for gossip messages,
containing payload + sender pseudonym + signature + TTL +
gossip_id.

**SMPL record.** Multi-writer Veilid DHT record schema. Specifies
member count and subkeys-per-member. With `o_cnt: 0`, all
subkeys belong to declared members — there are no owner-reserved
subkeys, and the creation keypair has no privileged write
access. The foundation of flat governance.

**Slot keypair.** Per-subkey Veilid keypair derived via
HKDF(`slot_seed`, `subkey_index`). Authorises a member to write
to a specific subkey. The `slot_seed` is shared with all
members via the invite, so any member can derive any slot's
keypair (which is fine because authenticity comes from the
pseudonym signature, not the slot keypair).

**STRIDE.** Threat-modelling taxonomy: Spoofing, Tampering,
Repudiation, Information Disclosure, Denial of Service,
Elevation of Privilege. Used in
[`security/threat-model.md`](security/threat-model.md).

**Strand Relay Network.** Friend-to-friend forwarding for cases
where direct routing is unreachable. Carol (mutual friend)
volunteers a dedicated relay route; Bob publishes it in his
profile; Alice sends through Carol who forwards an opaque
encrypted blob she cannot read. See
[`protocol/relay.md`](protocol/relay.md).

**Stronghold.** IOTA Stronghold — the on-disk vault that holds
long-term secrets (master secret, identity keys, Signal sessions,
MEK history, slot seed). Encrypted with a key derived via Argon2id
from the user's passphrase.

**Subkey.** A numbered entry within a Veilid DHT record. SMPL
records assign subkeys to specific writers. Each subkey holds up
to 32 KiB.

**Three-path delivery.** The chiral-network delivery model: every
operation travels three independent paths concurrently. Path 1 is
SMPL write (durability); Path 2 is gossip (low latency); Path 3 is
watch / inspect (consistency). Any single path succeeding is
sufficient. See
[`architecture/communities.md` §2](architecture/communities.md).

**Tier (1–7).** The crate hierarchy: Tier 1 types, Tier 2 secrets,
Tier 3 codec/records, Tier 4 route, Tier 5 gossip, Tier 6
governance, Tier 7 self-contained features. Lower tiers know
nothing about higher tiers. See
[`architecture/crates.md`](architecture/crates.md).

**TOFU (Trust On First Use).** The trust model for friend identity
keys: the first time we see a peer's identity key we accept it,
and any subsequent change requires re-verification. Optional
out-of-band verification is the strongest guarantee.

**Tombstone.** A CRDT entry that retroactively cancels a previous
entry (e.g. `Delete` cancels a message; `AdminDelete` cancels a
governance entry). Permanent — tombstones are kept after their
target is removed so subsequent merges still produce the right
result.

**TTL (Time To Live).** Two distinct uses: (1) the gossip envelope
hop count (TTL=5); (2) the Veilid DHT record TTL (~1 hour without
refresh). Distinct from the network-stack-level TTL.

**TUI (Text User Interface).** `rekindle-cli`'s ratatui-based
interactive screen UI, an alternative to one-shot CLI commands.
Toggleable via the `tui` cargo feature.

**UCred.** Process-level credentials extracted from a Unix domain
socket (`SO_PEERCRED` on Linux, `LOCAL_PEERCRED` on macOS): peer
PID and UID. Mixed into the Noise IK prologue on the IPC bus to
cryptographically bind OS-level identity to the encrypted
channel.

**VICE (Veilid Internet Connectivity Establishment).** Veilid's
NAT-traversal subsystem. Handles symmetric NAT detection, UDP
hole-punching, relay fallback, and protocol negotiation. Rekindle
deploys no STUN or TURN servers — VICE is the entire NAT-traversal
strategy.

**X25519.** Curve25519-based Diffie-Hellman key agreement. Used
for MEK wrapping, DM MEK derivation, call-key derivation, and
the daemon-track IPC bus DH within Noise IK.

**X3DH.** Extended Triple Diffie-Hellman, the Signal Protocol
session-establishment scheme. Combines identity key, signed
prekey, and one-time prekey to derive the initial root key.

**Xfire skin / Symbiosis.** The original Xfire skin (extracted to
`legacy/unpacked/skins/Symbiosis/`) used as visual reference for
Rekindle's UI. 460 GIF assets + XML layouts + `Themes.xml`. See
[`architecture/ui-skin.md`](architecture/ui-skin.md).

**Zeroize.** The Rust trait for explicitly wiping a value's bytes
from memory. Combined with `ZeroizeOnDrop` to ensure secrets are
wiped when their containing struct is dropped. Implemented for
every secret type in `rekindle-secrets`.
