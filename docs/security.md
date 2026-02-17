# Security Model

Rekindle implements a four-layer encryption architecture. Each layer addresses a
distinct threat surface, and all four operate simultaneously during normal
operation.

## Encryption Layer Stack

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 4: Stronghold At-Rest Encryption                     │
│  AES-256-GCM, Argon2id KDF                                  │
│  Scope: Private keys and secrets stored on device            │
├─────────────────────────────────────────────────────────────┤
│  Layer 3: Group Media Encryption Key (MEK)                   │
│  AES-256-GCM, per-channel symmetric key                      │
│  Scope: Community channel messages                           │
├─────────────────────────────────────────────────────────────┤
│  Layer 2: Signal Protocol (Double Ratchet)                   │
│  X3DH key agreement + symmetric ratchet                      │
│  Scope: 1:1 direct messages between friends                  │
├─────────────────────────────────────────────────────────────┤
│  Layer 1: Veilid Transport Encryption                        │
│  XChaCha20-Poly1305                                          │
│  Scope: All data in transit over the Veilid network          │
└─────────────────────────────────────────────────────────────┘
```

## Layer 1: Veilid Transport

**Algorithm:** XChaCha20-Poly1305
**Managed by:** Veilid core (automatic)

Veilid encrypts all data in transit between nodes. Safety routes provide sender
privacy by routing through multiple hops. Private routes provide receiver
privacy in the same manner.

This layer does not provide end-to-end encryption. Veilid nodes along the route
decrypt and re-encrypt at each hop. Forward secrecy is not guaranteed at this
level. Layers 2 and 3 exist specifically to address these gaps.

## Layer 2: Signal Protocol (1:1 Messages)

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
- One-time prekeys are consumed on first use (removed from DHT after fetch)
- Signed prekeys are rotated periodically
- If no one-time prekeys remain, X3DH proceeds with 3 DH operations instead of 4

### Plaintext Fallback

If no Signal session exists for a peer, or if decryption fails, inbound messages
are processed as plaintext with a `tracing::warn!()` log. The sender and receiver
have no UI indication that encryption was bypassed. This fallback exists to allow
friend request bootstrapping before a Signal session is established.

### Implementation

The `rekindle-crypto` crate provides `SignalSessionManager` which wraps the
signal protocol primitives with Stronghold-backed key storage. Session state is
persisted in the `signal_sessions` SQLite table. PreKeys are managed in the
`prekeys` table.

## Layer 3: Group Media Encryption Key (Channel Messages)

**Algorithm:** AES-256-GCM
**Managed by:** `rekindle-crypto` crate (`group/media_key.rs`)

**Implementation status:** The MEK encrypt/decrypt primitives are complete and
tested. The integration pipeline (Stronghold storage, Signal-session-based
distribution to members, per-message encryption in `send_channel_message`) is
not yet wired. Channel messages currently transmit as plaintext JSON over the
Veilid transport layer (Layer 1 encryption still applies).

Signal Protocol is designed for 1:1 sessions. Maintaining N*(N-1)/2 pairwise
sessions for large groups is impractical. Community channels will use a shared
symmetric key instead.

### MEK Lifecycle (Designed, Not Yet Integrated)

```
Channel creation:
  1. Admin generates random AES-256-GCM key (MEK)        ← implemented
  2. For each member: encrypt MEK with their Signal session  ← TODO
  3. Store encrypted MEK bundles in community DHT record     ← TODO

Member joins:
  1. Admin encrypts current MEK for new member's Signal session  ← TODO
  2. Updated MEK bundle pushed to DHT                            ← TODO

Member leaves or is removed:
  1. Generate new MEK (rotation)                            ← generates key only
  2. Re-encrypt new MEK for all remaining members           ← TODO
  3. Push updated MEK bundle to DHT                         ← TODO
  4. Old messages remain encrypted with old MEK

Message encryption:
  1. Sender encrypts message body with current MEK + random nonce  ← primitive exists
  2. Nonce + ciphertext sent to channel                            ← TODO (no MEK lookup)
  3. All members with current MEK can decrypt                      ← primitive exists
```

### Key Rotation Triggers (Planned)

- Member leaves or is removed (prevents reading future messages)
- Admin explicitly rotates (periodic security hygiene)
- Configurable time threshold elapsed

### Scalability

Community membership is capped at approximately 100 members for the initial
implementation. TreeKEM-based key distribution is planned for larger communities.

## Layer 4: Stronghold (At-Rest Encryption)

**Algorithm:** AES-256-GCM
**KDF:** Argon2id (memory-hard, GPU-resistant)
**Managed by:** `iota_stronghold` (used directly, not via Tauri plugin)

### Vault Contents

| Secret | Purpose |
|--------|---------|
| Ed25519 private key | Identity signing |
| X25519 private key | Diffie-Hellman key agreement |
| Signal identity keypair | Signal Protocol identity |
| Signed prekey (private) | Signal Protocol key exchange |
| One-time prekeys (private) | Signal Protocol first-contact |
| Community MEKs | Channel message decryption |

### Protection

- Vault encrypted with key derived from user's local passphrase via Argon2id
- Keys zeroized from memory when Stronghold is dropped
- Vault file never leaves the device
- Login is the act of providing the passphrase to unlock the Stronghold

Each identity has its own `.stronghold` file. Multiple identities can coexist on
one device, each with a separate passphrase.

## Identity System

```
Traditional:  username + password → central server validates → session token
Rekindle:     passphrase → unlock local vault → Ed25519 keypair = identity
```

- **Identity = Ed25519 keypair** generated locally on first run
- **Public key** is shared with friends, published to DHT
- **Private key** never leaves the device (stored in Stronghold)
- **Passphrase** unlocks the local vault — never transmitted
- **No recovery** — lose the passphrase, generate a new identity

### Trust Model

- **Trust on first use (TOFU)** — first contact establishes identity binding
- **Key verification** — optional out-of-band fingerprint comparison
- **No certificate authority** — no third party vouches for identity
- **Key continuity** — Signal session tracks identity key, warns on change

The `trusted_identities` SQLite table records identity keys seen for each peer,
with an optional `verified` flag for out-of-band confirmation.

### Adding Friends

```
1. Alice obtains Bob's public key (paste, QR code, invite link, deep link)
2. Alice sends FriendRequest via Veilid app_message to Bob's DHT route
   - Request includes: display name, PreKeyBundle, profile DHT key, route blob, mailbox key
3. Bob receives request, sees Alice's display name and public key
4. Bob accepts → both add each other to their DHT friend lists
   - Accept includes: PreKeyBundle, profile DHT key, route blob, mailbox key, session init info
5. Signal session established via PreKeyBundle exchange
6. Messaging begins
```

### Invite Links

Friend requests can also be initiated via Ed25519-signed invite blobs:
- `generate_invite()` creates a signed blob with identity info and PreKeyBundle
- Blob is base64url-encoded into a `rekindle://invite/{blob}` deep link
- Recipient verifies the Ed25519 signature before processing
- Prevents invite forgery — only the keypair owner can generate valid invites

### Block List

Incoming messages from blocked users are dropped at the `message_service`
layer before decryption or processing. The `blocked_users` SQLite table
tracks blocked public keys per identity.

### Community Pseudonyms

Users participate in communities under unlinkable pseudonyms derived via
HKDF-SHA256 from their master secret and the community ID. This provides:

- **Cross-community unlinkability** — different pseudonym in each community
- **Deterministic** — same user always gets the same pseudonym in a given community
- **No correlation** — observers cannot link pseudonyms to the user's real identity

## Threat Model

### Protected Against

| Threat | Protection |
|--------|------------|
| Network surveillance | Veilid transport encryption + routing privacy |
| Message interception | Signal Protocol end-to-end encryption |
| Server compromise | No server to compromise |
| Metadata collection | No central server logging connections |
| Stored data theft | Stronghold encryption at rest |
| Key compromise (past messages) | Signal forward secrecy |
| Key compromise (future messages) | Signal future secrecy via ratchet |
| Removed member reading future messages | MEK rotation on membership change |

### Not Protected Against

| Threat | Rationale |
|--------|-----------|
| Compromised device | Full OS compromise defeats all software defenses |
| Coerced passphrase disclosure | Passphrase unlocks all local secrets |
| Traffic analysis | Veilid routes reduce but do not fully prevent |
| Social engineering | User may share keys with wrong people |
| Large-scale Sybil attack on DHT | Veilid's DHT defenses are still maturing |
| Quantum computing | Ed25519/X25519 are not post-quantum (future work) |
