# Security Model

Rekindle implements a five-layer encryption architecture. Each layer
addresses a distinct threat surface, and all operate simultaneously
during normal operation.

## Encryption Layer Stack

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 5: Audit Chain (tamper-evident integrity)            │
│  BLAKE3-keyed MAC chain (rekindle-audit)                    │
│  Scope: Administrative actions logged for tamper-evidence   │
├─────────────────────────────────────────────────────────────┤
│  Layer 4: Vault At-Rest Encryption (rekindle-vault)         │
│  SQLCipher AES-256-CBC + per-entry AES-256-GCM,             │
│  Argon2id KDF → BLAKE3-keyed master derivation              │
│  Scope: Private keys and secrets stored on device           │
├─────────────────────────────────────────────────────────────┤
│  Layer 3: Group / DM Media Encryption Key (MEK)             │
│  AES-256-GCM, per-channel / per-DM symmetric key            │
│  Scope: Community channel messages, DMs, group DMs          │
├─────────────────────────────────────────────────────────────┤
│  Layer 2: Signal Protocol (Double Ratchet)                  │
│  PQXDH key agreement + Double Ratchet                       │
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

Voice traffic uses the same 3-hop Tor-class `SafetySelection::Safe` route as
every other path — the sender's node identity is never exposed, so voice gets
no weaker routing privacy than chat. It selects `Stability::LowLatency` within
that anonymity floor (the lowest-latency variant that is still anonymous), and
the mouth-to-ear budget is re-baselined to the ITU-T G.114 interactive band.
Voice packets are authenticated and the call-level signaling traverses the same
private-route path.

## Layer 2: Signal Protocol (1:1 Friend Messages)

**Algorithm:** PQXDH (rev. 3) key agreement + Double Ratchet
**Managed by:** `rekindle-crypto` crate (`signal/` module); sessions in `rekindle-db` (`ratchet_sessions`)

### Properties

| Property | Description |
|----------|-------------|
| Forward secrecy | Compromise of current keys cannot decrypt past messages |
| Future secrecy | Session self-heals after key compromise via ratchet advancement |
| Post-compromise security | Future messages are secured even after temporary key exposure |
| Deniability | Messages are not cryptographically attributable to sender |
| Asynchronous | Sessions can be established while the recipient is offline |

### Session Establishment (PQXDH)

```
Alice initiates a session with Bob:

1. Read Bob's prekey record (named in his profile's subkey 5), check every
   signature under the identity key she holds for him, and pick a slot
   ├── Bob's identity key (Ed25519, encoded as 0x06 ‖ key)
   ├── the head: his signed prekey (X25519) and PQ last-resort key (ML-KEM-768)
   └── one slot's one-time X25519 and ML-KEM prekeys (if one is unused)

2. Perform PQXDH (rev. 3) key agreement:
   DH1..DH3 (identity, signed prekey, ephemeral), DH4 with the one-time
   prekey when present, plus an ML-KEM-768 encapsulation to Bob's PQ key
   SK  = HKDF(DH1 ║ DH2 ║ DH3 ║ [DH4] ║ SS_kem)

3. Alice sends a PreKeyInit frame: her base key, the prekey ids used, the
   KEM ciphertext and a first payload encrypted under SK
4. Bob derives the same shared key from his private keys and answers
5. The Double Ratchet begins, one symmetric key per message
```

### Wire carriers

A 1:1 datagram is `'D' ‖ DmFrame` (`schemas/direct.capnp`); an untagged
datagram is dropped. A frame travels one of three ways:

- **Ratchet.** The spec Double Ratchet (`signal/{dr,record}`), one
  `SessionRecord` per peer in `ratchet_sessions`, stepped and committed in
  one DB transaction before any send. The associated data binds both
  identities and the frame typeId.
- **PreKeyInit.** Opens a session (above) and carries its first payload
  (`FriendAccept`, `SessionResetAccept`).
- **Sealed.** The pre-session kinds (`FriendRequest`, `FriendReject`,
  `PreKeyRejected`, `SessionResetRequest`/`Decline`, push-relay kinds) have
  no session yet.
  They are HPKE Base sealed to the recipient identity's X25519 key and
  Ed25519-signed over `DM_PRESESSION_SIG ‖ recipient ‖ typeId ‖ enc ‖
  ciphertext`, with a receive-side replay guard on `(sender, nonce)`.

There is no plaintext fallback: a frame that fails its carrier's checks, or
that fails to decrypt on an established session, is dropped and logged,
never re-initiated on its own. Decrypted plaintext is written to
`unprocessed` in the same transaction as the ratchet advance and replayed
at unlock, so a crash never loses a message whose key was spent.

### Serverless PreKey Distribution

Standard Signal relies on a central server to hand out prekeys. Rekindle
publishes them in a prekey record per device instead (plan E2.4):

- **The record.** A DFLT(101) Veilid DHT record with an owner keypair of its
  own. Subkey 0 (the head) holds the signed prekey and the ML-KEM-768
  last-resort key; subkeys 1..=100 each hold one slot, a one-time X25519 key
  and a one-time ML-KEM-768 key. Each value is capped at 10,381 B. Profile
  subkey 5 holds the device list (`PrekeyRecordList`: device id and record
  key; one entry until E5.3).
- **Ids and signatures.** Every id is a random non-zero u31, unique per kind.
  Every signature is the identity's Ed25519 over domain ‖ id ‖ key, with
  the head's `issued_at` signed too:
  `PREKEY_SPK_SIG ‖ u32be(spk_id) ‖ i64be(issued_at) ‖ EncodeEC(spk)`,
  `PREKEY_PQ_LR_SIG ‖ u32be(id) ‖ i64be(issued_at) ‖ EncodeKEM(pqpk)`,
  `PREKEY_PQ_OT_SIG ‖ u32be(id) ‖ EncodeKEM(pqpk)`,
  `PREKEY_OPK_SIG ‖ u32be(opk_id) ‖ EncodeEC(opk)`. A reader refuses a head
  dated more than 14 days from now either way (`SIGNED_MAX_AGE_MS`,
  `CryptoError::StalePrekeys`).
- **Maintenance (B1), automatic.** The "prekey maintenance" task on both
  hosts refills a consumed slot, fills every empty slot once fewer than 10
  are fresh, replaces a slot older than 90 days, and rotates the head every
  2 days, so no published head reaches 14 days while it runs. A replaced
  key stays usable for 30 days, then the archive sweep deletes it. At
  unlock, before the profile names the record, the own-records stage
  re-publishes every value the record does not hold and runs a first pass.
- **Pick (B2).** The initiator force-reads the device list and the record,
  keeps only what verifies under the pinned identity, and picks uniformly
  among the slots it has not used (`prekey_slots_used`). With none left,
  PQXDH proceeds without DH4 and the one-time KEM key (the last-resort KEM
  key is used).
- **Pin mismatch (B3).** A record not signed by the identity we hold for
  the peer opens nothing and changes no trust: it raises
  `IdentityPinMismatch` for the user.
- **Consumption (B8).** A one-time key is consumed only after the init's
  first message decrypts, in one transaction that erases its secret,
  records a last-resort use, stores the session and refills the slot; the
  refilled slot is written to the record after the commit.
- **Rejection and retry (B5–B7).** An init naming a consumed, unknown or
  archived key gets a sealed `PreKeyRejected` (0x0005) naming the ids and a
  hash of the initiator's base key, only to a sender the friend gate admits
  for that init, after a random 2–10 s delay, at most one per base key and
  ten per sender per 3 h. The initiator accepts it only if it names its
  unanswered init, then resends the same first contact once from a fresh
  slot (`HandshakeRetrying`); a second rejection fails visibly with Retry
  (`HandshakeFailed`).
- **Replay (B9).** A last-resort-only init whose `(lr_id, spk_id, base_key)`
  already opened a session is dropped silently.
- **Rotation.** An identity rotation mints the new identity's full set and
  publishes it into the same record before the `IdentityRotated` notices go.

Private prekey material lives in the `signal_prekeys` table
(`rekindle-db::prekeys`). `PrekeyStatus` (CLI `rekindle keys prekeys
status`, the GUI's debug `pqxdh_bundle_info`) shows the record and its
upkeep, and the status snapshot's `crypto.prekeys` check warns when fewer
than 10 slots are fresh or the head is older than 14 days.

### Safety numbers and key continuity

Safety numbers use libsignal's format (`rekindle-crypto::fingerprint`): 60
digits plus the scannable (QR) form, over the Ed25519 identity keys.
Verification state is kept in `signal_identities`. When a verified peer's
key changes, the change is recorded and escalates the next review
(`VerifiedKeyChanged`). A friend who rotates identity sends a signed
`IdentityRotated` frame over the old session
(`rekindle-transport::operations::friend::identity_rotation`), which the
receiver marks unverified.

### Implementation

The `rekindle-crypto` crate holds the Double Ratchet (`signal::dr`),
`SessionRecord` (`signal::record`) and PQXDH (`signal::pqxdh`) as pure
state machines. Persistence is in `rekindle-db`: `ratchet_sessions`,
`signal_prekeys`, `signal_identities` and `unprocessed`. The vault holds no
Signal entries. `SignalSessionManager` and the old non-spec ratchet are
gone (plan E2.3).

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
generation — receivers must keep historical MEKs because each message
carries its `mek_generation`. (Voice frames carry the generation's low
bits in their SFrame KID instead; see `architecture/voice.md`.)

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
has its own member registry.

There is no MEK vault. An earlier design published per-member wrapped
copies of the key into a registry subkey; under `o_cnt: 0` no writer is
credentialed for a community-wide subkey, and the key is never written
to the DHT at all. Distribution is per-recipient `app_call` from the
elected rotator.

## Layer 4: Vault (`rekindle-vault`, At-Rest Encryption)

**Algorithms:** SQLCipher AES-256-CBC at the page level **and** per-entry
AES-256-GCM (double encryption)
**KDF:** Argon2id (memory-hard, GPU-resistant) → BLAKE3 `derive_key` of
the master
**Managed by:** `rekindle-vault` (replaces the previous `iota_stronghold`
dependency — see [`../decisions/0006-vault-replaces-stronghold.md`](../decisions/0006-vault-replaces-stronghold.md))

### Vault Contents

| Secret | Purpose |
|--------|---------|
| Ed25519 private key | Identity signing |
| X25519 private key | Diffie-Hellman key agreement (DM MEK derivation) |
| Per-community / per-channel MEKs | Channel message decryption |
| Slot seed + slot keypairs | Per-community pseudonyms |
| Audit MAC key + tail anchor | Tamper-evident audit chain (Layer 5) |
| Veilid protected store key | Veilid local storage encryption |

### Protection

- Vault file (`{pubkey}.vault`) encrypted with a key derived from the
  user's local passphrase via Argon2id (64-byte master), then split with
  `blake3::derive_key` under three labels: `rekindle-vault-sqlcipher-v1`
  (the vault's SQLCipher page key), `rekindle-vault-entry-v1` (the
  per-entry AES-256-GCM key) and `rekindle-vault-app-db-v1` (the identity
  database's SQLCipher key) (`rekindle-vault/src/store.rs`).
- The Argon2id parameters and the 32-byte salt live in the 64-byte
  `{pubkey}.vault.hdr` header (plaintext — neither is secret, only unique
  per vault; `rekindle-vault/src/header.rs`).
- Keys zeroised from memory when the vault handle is dropped.
- Vault file never leaves the device.
- Login is the act of providing the passphrase to unlock the vault.

Ratchet sessions and prekey secrets are not vault entries: they live in
the identity database `state/{pk}.db` (`ratchet_sessions`,
`signal_prekeys`), SQLCipher under the `rekindle-vault-app-db-v1` key, so
they open only after unlock. An identity export carries the registry
entry, the vault and its header, never that database
(`rekindle-node/src/host/identity/transfer.rs`), so sessions and prekeys
never leave the device.

Each identity has its own vault file. Multiple identities can coexist on
one device, each with a separate passphrase.

## Layer 5: Audit Chain (`rekindle-audit`)

**Algorithm:** BLAKE3-keyed MAC chain.
**Managed by:** `rekindle-audit` + `rekindle_node::host::audit` (both hosts).

Every administrative action (community / channel / role / member
mutations, MEK rotations, vault passphrase changes, device pairings) is
appended to a tamper-evident chain. Each `AuditEntry` carries `prev_mac +
mac` where `mac = BLAKE3-keyed(key, prev_mac || cursor_le ||
payload_json)`. Tampering with any byte of any entry's payload
invalidates every entry from that cursor forward.

The MAC key lives in the vault under `("audit", "mac_key")`. Loss of the
vault loses the chain; theft of the vault gains MAC-forgery capability —
no worse than what the attacker already has by virtue of holding the
identity secret. See [`../architecture/audit-chain.md`](../architecture/audit-chain.md).

## Identity System

```
Traditional:  username + password → central server validates → session token
Rekindle:     passphrase → unlock local vault → Ed25519 keypair = identity
```

- **Identity = Ed25519 keypair** generated locally on first run
- **Public key** is shared with friends, published to DHT
- **Private key** never leaves the device (stored in the vault)
- **Passphrase** unlocks the local vault — never transmitted
- **No recovery from passphrase loss** — but cross-device sync (architecture
  §28.4) lets you pair a new device while you still have access to an
  existing one.

### Trust Model

- **Trust on first use (TOFU)** — first contact establishes identity binding
- **Key verification** — optional out-of-band fingerprint comparison
- **No certificate authority** — no third party vouches for identity
- **Key continuity** — the `signal_identities` table records each peer's identity key and verification; a change of a verified key escalates review (`VerifiedKeyChanged`)

### Adding Friends

```
1. Alice obtains Bob's public key (paste, QR code, invite link, deep link)
2. Alice sends FriendRequest via Veilid app_message to Bob's DHT route
   - Request includes: display name, profile DHT key, route blob,
     mailbox key, optional invite_id (no prekeys)
3. Bob receives request, sees Alice's display name and public key
4. Bob accepts → both add each other to their DHT friend lists
   - Bob reads Alice's prekey record from the profile key the request
     named and picks a slot
   - Accept (profile/route/mailbox keys) is the encrypted first payload
     of Bob's PreKeyInit, which names the prekey ids used
5. Session established by that PreKeyInit (PQXDH)
6. Messaging begins
```

### Invite Links

Friend requests can also be initiated via Ed25519-signed `InviteBlob`s:
- `create_invite_blob()` creates a signed blob with identity info, profile/
  route/mailbox keys, and an `invite_id` correlation token (no prekeys)
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
| Stored data theft | Vault (`rekindle-vault`) encryption at rest |
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
