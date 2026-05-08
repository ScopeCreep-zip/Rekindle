# Privacy Properties

This document maps the boundary between **what Veilid provides**,
**what Rekindle adds**, and **what is not protected**. Read alongside
[`overview.md`](overview.md) (encryption layer stack),
[`threat-model.md`](threat-model.md) (adversary model), and
[`crypto-primitives.md`](crypto-primitives.md).

## What "private" means here

Privacy is decomposed into the dimensions an adversary might want to
correlate. We use the LINDDUN privacy taxonomy informally:

- **Linkability** — can two events be tied to the same actor?
- **Identifiability** — can an actor be tied to a real-world person?
- **Detectability** — can an event be detected as Rekindle traffic?
- **Disclosure** — can content be read by the wrong party?
- **Awareness** — does the user know what is being shared?

Each property below is graded **what Veilid gives**, **what we add**,
and **what we do not promise**. Properties marked *open* are tracked
in [`../roadmap.md`](../roadmap.md).

## 1. What Veilid provides

### 1.1 Hop-by-hop transport encryption

Veilid encrypts every packet between adjacent network nodes with
XChaCha20-Poly1305. A passive observer of the wire sees only Veilid
ciphertext.

**Caveat:** this is **hop-by-hop**, not end-to-end. Each Veilid relay
in the path decrypts and re-encrypts. A relay node sees plaintext at
the Veilid layer. Rekindle therefore does **not** rely on Veilid
transport for content confidentiality — it is the outermost shell of
defense in depth, and the application-layer ciphertext (Signal / MEK /
FEK) is what actually protects content.

### 1.2 Sender anonymity (safety routes)

`SafetySelection::Safe(SafetySpec { hop_count, … })` builds a
multi-hop forward route. Each hop along the route knows only the
adjacent nodes — no single intermediate node knows both the sender's
identity and the destination. This is Tor-style sender anonymity built
into Veilid.

| Hop count | Latency | Anonymity |
|-----------|---------|-----------|
| 0 (`Unsafe`) | ~50 ms | None — sender visible |
| 1 | ~100 ms | One relay knows the sender |
| 2 | ~150 ms | No single node knows both ends |
| 3 (default) | ~200 ms | Strong — Tor-class |

Rekindle uses different hop counts per traffic type: voice uses
`Unsafe` for sub-50 ms latency (acceptable because voice channel
participants are mutually known); chat / governance use 1–2 hops; the
user can configure hop count for sensitive workflows.

### 1.3 Receiver anonymity (private routes)

A node creates a `private_route` and publishes the route blob. Anyone
holding the blob can send to the route, but **only the originator's
node** knows that blob → real network identity. The publisher's IP /
node identity is never revealed.

Routes expire (~120 s of inactivity); the presence service periodically
refreshes route blobs in the registry, which doubles as a liveness
signal.

### 1.4 NAT traversal without third-party servers (VICE)

Veilid Internet Connectivity Establishment handles symmetric NAT
detection, UDP hole-punching, relay fallback through the network's
relay node population, and protocol negotiation. **Rekindle deploys no
STUN, TURN, or relay servers.** All connectivity is via Veilid's
existing relay node population — every Rekindle instance contributes
relay capacity to the pool by default, and benefits from it
reciprocally.

### 1.5 No anonymous DHT writes

Every DHT write is **signed** by the writer's keypair. There is no
"write to this DHT key from nowhere" — the DHT enforces writer
authorisation at the schema layer. This is why we can run SMPL records
with `o_cnt: 0` and still know which subkey came from which member.

### 1.6 What Veilid does NOT do

- **End-to-end content encryption.** Veilid relay nodes decrypt and
  re-encrypt; we add Signal / MEK / FEK on top.
- **At-rest encryption of DHT values.** DHT replicas store ciphertext
  only because we encrypt before writing. Without our application
  encryption, the DHT would store plaintext.
- **Group messaging primitive.** Veilid has no built-in multicast or
  group abstraction. Rekindle's three-path delivery + gossip mesh +
  CRDT merge fills this gap.

## 2. What Rekindle adds

### 2.1 Pseudonym unlinkability across communities

Each community produces a different Ed25519 pseudonym derived via
HKDF(`master_secret`, `community_id`). Two communities yield
cryptographically unrelated pseudonyms: an attacker who compromises
one community's member list cannot link those identities to any other
community's member list.

**Caveat.** A user who voluntarily reveals "I am `pseudA` in
community A and `pseudB` in community B" provides the linkage
themselves. Rekindle provides no implicit linkage and does not offer
features (OAuth2, connected accounts, server discovery directories)
that would create one.

### 2.2 End-to-end content encryption

Every byte of user content is encrypted by Rekindle before it touches
the DHT or `app_message` transport:

- 1:1 friend messages → Signal Protocol per-message keys
- DM / group DM messages → ECDH-derived MEK + ratchet
- Community channel messages → per-channel MEK
- File chunks → per-file FEK (wrapped under channel MEK)
- Voice frames → per-channel MEK or per-call key
- Cross-device sync subkeys → per-subkey HKDF-derived key

Layer 4 in the stack — see [`overview.md`](overview.md).

### 2.3 Forward secrecy

- **1:1 messaging:** Signal's Double Ratchet — every message is
  encrypted with a unique key derived from the ratchet state. A
  long-term key compromise does not retrospectively decrypt prior
  messages.
- **Communities:** MEK rotation on every member departure. A departed
  member cannot decrypt content sent after their departure.
- **Voice channels:** MEK rotates on every join and every leave —
  late joiners cannot decrypt earlier frames; departing participants
  cannot decrypt later frames.
- **DM:** ratchet step every 100 messages or 24 hours.

### 2.4 Reader-validates governance

The CRDT merge engine enforces permissions on the *reader* side, not
the writer side. A peer with no `MANAGE_CHANNELS` permission *can*
write a `ChannelCreated` entry; honest readers ignore it. This means:

- A misbehaving client that ignores permission rules corrupts only its
  own view.
- There is no privileged writer who can be coerced into writing forged
  governance.
- Banned peers continue to "write" garbage but have no effect on
  honest peers.

### 2.5 Notification-only gossip

Gossip carries `MessageNotification { channel_id, subkey_index,
message_id, lamport_ts, sequence, content_hash }` — metadata, not
ciphertext. The actual MEK-encrypted message lives in the SMPL DHT
record; recipients fetch it from the DHT (5 replicas) rather than from
the gossip mesh (50–100+ nodes). This dramatically reduces the
ciphertext-distribution surface available to harvest-now-decrypt-later
adversaries. See [`../architecture/communities.md` §2](../architecture/communities.md#2-three-path-delivery).

### 2.6 Strand Relay for unreachable peers

When direct routing fails, friends forward an opaque envelope through
a mutual friend. The relay friend cannot read content (encrypted to
the recipient's key); the sender cannot identify which friend is
relaying (opaque pool with dummy padding); the recipient cannot link
the relay-routed delivery back to the sender's failed direct attempts
(different routes, different timing). See [`../protocol/relay.md`](../protocol/relay.md).

### 2.7 Bundled, local-only game database

Game detection uses a JSON database that ships with the binary — no
remote service is queried. Output goes only to friends with explicit
presence visibility. The user's process list never leaves the device.

### 2.8 Opt-in, content-free push relay

Mobile push relay sends only `{type: "wake", ts}` — no content, no
record-key reference, no community ID. The platform vendor (Apple,
Google) sees only that the device might have had something happen at
time T. The relay daemon sees which DHT keys the device cares about
(metadata leak), which is the cost of timely notification on a fully
suspended device. The feature is **opt-in** and **self-hostable** so
users can decide their own threshold.

### 2.9 No telemetry

There is no analytics, crash reporting, A/B testing, engagement
tracking, or "diagnostic ping" of any kind. The app does not call out
to scopecreep.zip or anywhere else for any reason during normal
operation. The only outbound traffic is Veilid network bootstrap,
peer connections, and explicit user actions.

### 2.10 Memory hygiene

Every secret type implements `Zeroize + ZeroizeOnDrop`. The Stronghold
vault is sealed when not in use. The CLI's `print_stdout` is denied at
the lint level — every output goes through structured renderers so
secrets cannot accidentally leak via formatted strings.

## 3. What is NOT protected

These limits are documented in [`threat-model.md`](threat-model.md);
they are restated here to be unambiguous about the privacy floor.

### 3.1 Pre-departure content cached by a member

Forward secrecy protects only future content. A member who cached a
message before being banned still has the ciphertext, and they had
the MEK at the time, so they can decrypt their cache. There is no
"erase the past" — once bytes are delivered, they exist. This is
inherent to E2E P2P.

### 3.2 Live memory of a running, unlocked app

Once the user authenticates and the Stronghold vault is open, secrets
are in memory. A live memory dump (root-level malware, debugger
attach, kernel exploit) can extract them. Zeroize-on-drop limits the
exposure window for *not currently in use* secrets but cannot
eliminate live ones.

### 3.3 Social graph visible to community members

Inside a community, every member knows every other member's
pseudonym — that is what makes the gossip mesh work. The pseudonym is
unlinkable to the user's identity in other communities, but within
this community, identifiability between two pseudonyms is
non-repudiable. Voluntarily revealing "I am also pseudonym X in
community Y" creates the linkage.

### 3.4 Timing analysis on the wire

A network adversary observing many Veilid relays can do
traffic-pattern analysis: when did the user send messages, how long
were the bursts, who else's messages happened to be active at the
same time. Veilid's safety routes raise the cost; high-hop counts
raise it further. We do not pad traffic to a constant rate.

### 3.5 App-installation visibility on mobile

Push notifications go through Apple / Google. Even with content-free
wake pushes, the *fact* that this device is using Rekindle is
observable to the platform vendor. Users with this concern run
Tier 2 only or use the Tauri desktop build instead.

### 3.6 Side channels in cryptographic primitives

We use constant-time implementations from `ed25519-dalek`,
`x25519-dalek`, `aes-gcm`, etc. Hardware-level side channels (cache
timing, branch prediction, EM emanation) on shared hosts are out of
scope — this is the same caveat every cryptographic library carries.

### 3.7 Coerced unlock

Rekindle has no plausible-deniability vault, no duress passphrase, no
"second password" feature. A user coerced into unlocking their device
hands over everything the unlocked app can access. Mitigations are
device-level, not application-level.

### 3.8 Master-secret rotation (open work)

A paired device that unpairs retains the master secret. Master-secret
rotation is an open design problem because rotating it invalidates
every per-community pseudonym derived from it (cross-community
pseudonym continuity is one of our *features*, and rotation breaks
it). This is a known limitation tracked in [`../architecture/sync.md`](../architecture/sync.md).

### 3.9 Reproducible builds (open work)

Until reproducible-build CI lands, distribution binaries cannot be
audited byte-for-byte against the source tree. Source-built copies
(`cargo install --path .`) are verifiable; downloaded artifacts are
trust-the-publisher.

### 3.10 Veilid-level metadata

Veilid's safety / private routes hide identity, but the **fact** that
some node is running Veilid is visible. ISP-level observers can see
"this IP is talking to Veilid relay nodes." Bridges / pluggable
transports for Veilid are upstream work, not Rekindle's.

## 4. Threat-model boundaries at a glance

| Concern | We protect against | We do not promise against |
|---------|-------------------|--------------------------|
| Reading message content | Network observers, DHT relay nodes, the Veilid network itself, push-relay vendors | Adversaries with live memory access on an unlocked device |
| Linking the user across communities | Curious peers, network observers, mass surveillance | Voluntary disclosure by the user |
| Detecting that a Rekindle community exists | Without the DHT key | Operators of relay nodes carrying a known pattern of traffic to the community's records |
| Detecting that this device runs Rekindle | Network observers (mostly — pluggable transports are upstream) | Mobile push platform vendors when Tier 3 is enabled |
| Tampering with messages | Network adversaries, malicious peers, banned members | A peer who replaces a chunk in *their own* cache and refuses to serve it (loss of availability, not integrity) |
| Mass spam | Per-sender rate limit + governance ban | A sustained botnet attack on the gossip mesh |
| Identity theft | Out-of-band verification + TOFU | Initial impersonation before key exchange happens (the user must verify out-of-band for the strongest guarantee) |
| Coerced unlock | None | Coerced unlock |
| Side channels | Constant-time crypto implementations | Hardware-level side channels |
| Long-term key compromise | Forward secrecy via Signal Double Ratchet + MEK rotation | Past sessions where the key was active |

## 5. References

- [`overview.md`](overview.md) — encryption-layer stack
- [`threat-model.md`](threat-model.md) — adversary model and STRIDE/LINDDUN
- [`crypto-primitives.md`](crypto-primitives.md) — primitive selection
- [`../architecture/communities.md`](../architecture/communities.md) — chiral-network properties
- [`../protocol/relay.md`](../protocol/relay.md) — strand and push relay properties
- [Veilid developer book](https://veilid.gitlab.io/developer-book/)
- [Signal Protocol specifications](https://signal.org/docs/)
