# Security Model

Rekindle implements a four-layer encryption architecture. Each layer addresses
a distinct threat surface, and all four operate simultaneously during normal
operation.

## Encryption Layer Stack

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 4: Stronghold At-Rest Encryption                     │
│  AES-256-GCM, Argon2id KDF                                  │
│  Scope: Private keys and secrets stored on device           │
├─────────────────────────────────────────────────────────────┤
│  Layer 3: Group / DM Media Encryption Key (MEK)             │
│  AES-256-GCM, per-channel / per-DM symmetric key            │
│  Scope: Community channel messages, DMs, group DMs          │
├─────────────────────────────────────────────────────────────┤
│  Layer 2: Signal Protocol (Double Ratchet)                  │
│  X3DH key agreement + symmetric ratchet                     │
│  Scope: 1:1 friend messages                                 │
├─────────────────────────────────────────────────────────────┤
│  Layer 1: Veilid Transport Encryption                       │
│  XChaCha20-Poly1305                                         │
│  Scope: All data in transit over the Veilid network         │
└─────────────────────────────────────────────────────────────┘
```

All raw key material is confined to one crate — `rekindle-secrets`. Every
secret type implements `Zeroize + ZeroizeOnDrop`. No other crate in the
workspace imports `ed25519-dalek`, `x25519-dalek`, `aes-gcm`, or `hkdf`
directly.

## Layer 1: Veilid Transport

**Algorithm:** XChaCha20-Poly1305
**Managed by:** Veilid core (automatic)

Veilid encrypts all data in transit between nodes. Safety routes provide
sender privacy by routing through multiple hops. Private routes provide
receiver privacy in the same manner.

This layer does not provide end-to-end encryption — Veilid nodes along the
route decrypt and re-encrypt at each hop. Forward secrecy is not guaranteed
at this level. Layers 2 and 3 exist to address those gaps.

Voice traffic uses `SafetySelection::Unsafe` for direct UDP-like delivery,
trading routing privacy for latency. Voice packets are still authenticated
and the call-level signaling traverses the standard private-route path.

## Layer 2: Signal Protocol (1:1 Friend Messages)

**Algorithm:** X3DH key agreement + Double Ratchet
**Managed by:** `rekindle-crypto` crate (`signal/` module)

### Properties

| Property | Description |
|----------|-------------|
| Forward secrecy | Compromise of current keys cannot decrypt past messages |
| Future secrecy | Session self-heals after key compromise via ratchet advancement |
| Post-compromise security | Future messages are secured even after temporary key exposure |
| Deniability | Messages are not cryptographically attributable to sender |
| Asynchronous | Sessions can be established while the recipient is offline |

### Session Establishment (X3DH)

```
Alice initiates a session with Bob:

1. Fetch Bob's PreKeyBundle from his DHT profile (subkey 5)
   ├── Bob's identity key (Ed25519 → X25519 conversion)
   ├── Bob's signed prekey
   └── Bob's one-time prekey (if available)

2. Perform X3DH key agreement:
   DH1 = DH(Alice_identity,  Bob_signed_prekey)
   DH2 = DH(Alice_ephemeral, Bob_identity)
   DH3 = DH(Alice_ephemeral, Bob_signed_prekey)
   DH4 = DH(Alice_ephemeral, Bob_one_time_prekey)   [if available]
   SK  = HKDF(DH1 ║ DH2 ║ DH3 ║ DH4)

3. Alice sends initial message with her identity key + ephemeral key
4. Bob derives the same shared key from his private keys
5. Double Ratchet begins — every message uses a new symmetric key
```

### Serverless PreKey Distribution

Standard Signal relies on a central server to store PreKeyBundles. Rekindle
publishes them to Veilid DHT subkey 5 instead:

- Each user generates and publishes a PreKeyBundle to their DHT profile
- One-time prekeys are consumed on first use
- Signed prekeys are rotated periodically
- If no one-time prekeys remain, X3DH proceeds with 3 DH operations instead of 4

PreKey rotation and one-time prekey replenishment is the remaining open
work in Phase 2.

### Plaintext Fallback

If no Signal session exists for a peer, or if decryption fails, inbound
messages are processed as plaintext with a `tracing::warn!()` log. This
fallback exists to allow friend-request bootstrapping before a Signal session
is established.

### Implementation

The `rekindle-crypto` crate provides `SignalSessionManager` which wraps the
Signal protocol primitives with Stronghold-backed key storage. Session state
is persisted in the `signal_sessions` SQLite table. PreKeys are managed in
the `prekeys` table.

## Layer 3: Group / DM Media Encryption Key (MEK)

**Algorithm:** AES-256-GCM
**Managed by:** `rekindle-secrets::mek` (key material) +
`rekindle-crypto::group::media_key` (encrypt/decrypt)

Signal Protocol is designed for 1:1 sessions — maintaining N*(N-1)/2 pairwise
sessions for large groups is impractical. Communities and DMs use shared
symmetric keys instead.

### Per-Channel MEK (Communities)

Each channel has its own MEK (the older per-community MEK is still tracked
in `AppState.mek_cache` during the transition; new code uses the per-channel
`channel_mek_cache`). MEKs are distributed via the SMPL member-registry MEK
vault — one entry per member slot, encrypted to that member's pseudonym key.

### Per-DM MEK (Architecture §27)

For 2-party DMs the MEK is derived deterministically via X25519 ECDH between
the two identity keys — no separate key exchange round-trip is needed. For
group DMs the MEK is wrapped per recipient with X25519 (since ECDH is
pairwise) and distributed in the `GroupDmInvite` payload. The
`AppState.dm_mek_cache` holds the genesis MEK plus every materialized
generation — receivers must keep historical MEKs because each envelope
carries its `mek_generation`.

### MEK Rotation

The deterministic rotator in `rekindle-secrets::rotator` selects the peer
responsible for re-wrapping when membership changes. Selection is
`blake3(departed_pseudonym ‖ self_pseudonym)` — lowest hash wins. Triggers:

- Member leaves or is removed (prevents reading future messages)
- Admin explicitly rotates (periodic security hygiene)
- Configurable time threshold elapsed

Old messages remain decryptable with the old MEK; new messages use the new
generation.

### Scalability

The 255-subkey SMPL layout caps a single segment at 255 members. Beyond that,
**Plate Gates** (architecture §15) add fractal SMPL segments. Each segment
has its own member registry and MEK vault.

## Layer 4: Stronghold (At-Rest Encryption)

**Algorithm:** AES-256-GCM
**KDF:** Argon2id (memory-hard, GPU-resistant)
**Managed by:** `iota_stronghold` (used directly, not via Tauri plugin)

### Vault Contents

| Secret | Purpose |
|--------|---------|
| Ed25519 private key | Identity signing |
| X25519 private key | Diffie-Hellman key agreement (DM MEK derivation) |
| Signal identity keypair | Signal Protocol identity |
| Signed prekey (private) | Signal Protocol key exchange |
| One-time prekeys (private) | Signal Protocol first-contact |
| Per-community / per-channel MEKs | Channel message decryption |
| Veilid protected store key | Veilid local storage encryption |

### Protection

- Vault encrypted with key derived from user's local passphrase via Argon2id
- Keys zeroized from memory when Stronghold is dropped
- Vault file never leaves the device
- Login is the act of providing the passphrase to unlock the Stronghold

Each identity has its own `.stronghold` file. Multiple identities can coexist
on one device, each with a separate passphrase.

## Identity System

```
Traditional:  username + password → central server validates → session token
Rekindle:     passphrase → unlock local vault → Ed25519 keypair = identity
```

- **Identity = Ed25519 keypair** generated locally on first run
- **Public key** is shared with friends, published to DHT
- **Private key** never leaves the device (stored in Stronghold)
- **Passphrase** unlocks the local vault — never transmitted
- **No recovery from passphrase loss** — but cross-device sync (architecture
  §28.4) lets you pair a new device while you still have access to an
  existing one.

### Trust Model

- **Trust on first use (TOFU)** — first contact establishes identity binding
- **Key verification** — optional out-of-band fingerprint comparison
- **No certificate authority** — no third party vouches for identity
- **Key continuity** — Signal session tracks identity key, warns on change

The `trusted_identities` SQLite table records identity keys seen for each
peer, with an optional `verified` flag for out-of-band confirmation.

### Adding Friends

```
1. Alice obtains Bob's public key (paste, QR code, invite link, deep link)
2. Alice sends FriendRequest via Veilid app_message to Bob's DHT route
   - Request includes: display name, PreKeyBundle, profile DHT key,
     route blob, mailbox key, optional invite_id
3. Bob receives request, sees Alice's display name and public key
4. Bob accepts → both add each other to their DHT friend lists
   - Accept includes: PreKeyBundle, profile/route/mailbox keys, X25519
     ephemeral key, signed-prekey + one-time-prekey IDs
5. Signal session established via PreKeyBundle exchange
6. Messaging begins
```

### Invite Links

Friend requests can also be initiated via Ed25519-signed `InviteBlob`s:
- `create_invite_blob()` creates a signed blob with identity info, profile/
  route/mailbox keys, PreKeyBundle, and an `invite_id` correlation token
- The blob is base64url-encoded into a `rekindle://invite/{blob}` deep link
- The recipient verifies the Ed25519 signature before processing
- The `outgoing_invites` SQLite table tracks each issued invite by status
  so the buddy list can show pending invitations and the sender can revoke

### Block List

Incoming messages from blocked users are dropped at the `message_service`
layer before decryption or processing. The `blocked_users` SQLite table
tracks blocked public keys per identity. After block/unfriend the user
rotates their profile DHT key and notifies remaining friends via
`ProfileKeyRotated`.

### Community Pseudonyms

Users participate in communities under unlinkable pseudonyms derived via
HKDF-SHA256 from their master secret and the community ID:

- **Cross-community unlinkability** — different pseudonym in each community
- **Deterministic** — same user always gets the same pseudonym in a given
  community
- **No correlation** — observers cannot link pseudonyms to the user's real
  identity

## Cross-Device Sync

The personal sync record (architecture §28.4) is encrypted and lives on
the user's own device-list public keys. Pairing uses a short-lived code +
salt persisted in `pending_pairings`. The new device joins the device list
in subkey 3 of the personal record; once added, both devices can read/write
the shared sync state (manifest, read state, preferences, paired devices).

## Mobile Push Relay (Architecture §17.3 Tier 3)

A headless `veilid-server` push relay watches a list of DHT record keys on
a mobile device's behalf and sends content-free wake signals (`{"t":"wake"}`)
via FCM/APNs. The relay never sees ciphertext or metadata — only that
**some** registered record fired. The mobile client wakes, fetches the
relevant records itself, and decrypts locally.

## Threat Model

### Protected Against

| Threat | Protection |
|--------|------------|
| Network surveillance | Veilid transport encryption + routing privacy |
| Message interception (1:1) | Signal Protocol end-to-end encryption |
| Message interception (channels/DMs) | Per-channel / per-DM MEK encryption |
| Server compromise | No server to compromise |
| Push-relay compromise (mobile) | Relay sees only opaque wake signals |
| Metadata collection | No central server logging connections |
| Stored data theft | Stronghold encryption at rest |
| Key compromise (past messages) | Signal forward secrecy + MEK rotation |
| Key compromise (future messages) | Signal future secrecy + MEK rotation |
| Removed member reading future messages | MEK rotation on membership change |
| Cross-community correlation | Per-community pseudonym (HKDF) |

### Not Protected Against

| Threat | Rationale |
|--------|-----------|
| Compromised device | Full OS compromise defeats all software defenses |
| Coerced passphrase disclosure | Passphrase unlocks all local secrets |
| Traffic analysis | Veilid routes reduce but do not fully prevent |
| Social engineering | User may share keys with wrong people |
| Large-scale Sybil attack on DHT | Veilid's DHT defenses are still maturing |
| Quantum computing | Ed25519/X25519 are not post-quantum (future work) |
