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
| Z3 | Signal Protocol session state | High | Vault (per-peer) |
| Z4 | Pre-keys + signed prekey | Medium | Vault; published via DHT |
| Z5 | Channel MEK (per channel, current generation) | High | Memory cache; historical generations in the vault |
| Z6 | DM MEK chain | High | Vault |
| Z7 | Slot keypair (per community) | Low — derived deterministically; not sensitive | Derived from `slot_seed` on demand |
| Z8 | Slot seed | High — distributed via invite | Vault |
| Z9 | Master secret (cross-device) | Critical | Vault; transferred only via paired-device handshake |
| Z10 | SQLite contents (messages, friends, communities, etc.) | High — represents social graph | Disk; OS-level encryption is the user's choice |
| Z11 | Local Veilid node identity | Medium | Veilid storage; relinking requires bootstrap |
| Z12 | Friend list, community membership | High — represents who the user knows | DHT (encrypted) + SQLite |
| Z13 | Presence (status, route blob, game info) | Medium | DHT presence record (visible to friends/community members) |
| Z14 | Plaintext message content | Critical | Memory only; never persisted unencrypted |
| Z15 | Audit MAC key + tail anchor | High — tamper-evidence root | Vault; loss invalidates the chain |

## 3. STRIDE Threats and Mitigations

### S — Spoofing

| ID | Threat | Mitigation |
|----|--------|------------|
| S1 | Active attacker impersonates a friend in a 1:1 chat | Signal Protocol X3DH binds session to identity-key signatures; trust-on-first-use (TOFU) is the default with optional out-of-band verification. Identity-key changes surface a re-verification prompt, never auto-accept. |
| S2 | Banned member writes to their SMPL subkey claiming to be admin | Reader-validates: every reader recomputes `effective_permissions(writer, channel)` from the merged CRDT state. A writer without `MANAGE_*` permission has their entry silently dropped. (See [`../architecture/communities.md` §9](../architecture/communities.md#9-permissions).) |
| S3 | Attacker sends gossip envelopes claiming to be from another sender | Every envelope is Ed25519-signed by the sender's pseudonym. Recipients verify before processing. Forged signatures fail verification and are dropped. |
| S4 | Attacker spoofs the daemon over the IPC bus | Noise IK handshake binds the connection to the daemon's pre-known static key. The socket is owner-only (`0700` directory, `0600` socket; owner-only DACL on Windows), a peer whose UID differs from the daemon's is refused (`SO_PEERCRED`; `getpeereid` on macOS), and both UIDs plus the socket path are in the Noise prologue, so a handshake relayed to another user's or another path's bus fails. The boundary is the OS user: a process running as the same user is trusted to connect, and its clearance comes from the registry (plan D1). |
| S5 | Attacker spoofs an MEK-rotation `MEKGenerationBump` | Reader-validates the rotator: `MEKGenerationBump` carries `trigger_departed` and `cascade_skipped`; every reader recomputes the deterministic rotator and checks the writer matches. Forged bumps are silently dropped. |

### T — Tampering

| ID | Threat | Mitigation |
|----|--------|------------|
| T1 | Network adversary modifies a message in transit | Layer 1 Veilid transport encryption (XChaCha20-Poly1305) provides AEAD; tampering breaks decryption. Layer 3 Ed25519 signature on every envelope detects sender-end tampering. Layer 4 AES-256-GCM AEAD with content-bound AAD detects ciphertext tampering before decryption. |
| T2 | Curious peer modifies a stored chunk in their cache and serves the modified version | Layer 4 per-chunk SHA-256 verification on receive. Mismatches are dropped and the chunk is re-fetched from a different peer (swarm fetch). |
| T3 | Attacker modifies an `AttachmentOffer` in transit | Offer is signed by sender. Recipients additionally recompute the flat-list Merkle root over `chunk_hashes` and compare against the announced `merkle_root` to defend against serialization-confusion attacks. |
| T4 | Compromised peer writes garbage to a subkey to corrupt the CRDT view | Writes only affect the writer's own subkey. Garbage entries fail validation (`validate.rs`) or permission checks and are dropped. The CRDT merge is deterministic; honest peers converge regardless of what one peer wrote. |
| T5 | DHT routing node tampers with stored values | Veilid stores values across multiple replicas; Veilid's transport AEAD covers writes. Application-layer signing and AEAD provide the actual integrity guarantee. |
| T6 | Supply-chain attack via tampered dependency | `Cargo.lock` pinning, hash verification on `cargo install`, no `[patch.crates-io]` overrides for security-critical crates. Reproducible-builds doc tracks deterministic build verification (deferred — see open work). |

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
| I3 | Banned member decrypts post-ban messages | MEK rotation on member departure. New MEK is wrapped only for remaining members. |
| I4 | Banned member decrypts pre-ban messages they had once cached | Acknowledged limitation. Forward secrecy protects only future content. The MEK rotation cadence and the deterministic rotator protocol minimise the exposure window for *future* content. |
| I5 | Veilid relay node sees content | Layer 1 Veilid AEAD makes intermediate relays see only ciphertext. Layer 4 application AEAD makes them see only outer-envelope ciphertext even after Layer 1 unwraps. Defense in depth. |
| I6 | Attacker reads at-rest secrets from disk | Layer 4: `rekindle-vault` SQLCipher double-encryption — page-level AES-256-CBC keyed by Argon2id-derived master, per-entry AES-256-GCM on top, sealed when not in use. SQLite (main DB) is not encrypted by default — users on full-disk-encryption OSes get coverage from there; users without FDE accept this gap. |
| I7 | Memory dump on running app reveals secrets | Limited mitigation: every secret type implements `Zeroize + ZeroizeOnDrop` so secrets are wiped when the holding struct drops. Live secrets in active use are still in memory and recoverable from a memory dump. |
| I8 | Attacker correlates a user's pseudonyms across communities | Pseudonyms are derived per-community via HKDF(`master_secret`, `community_id`). Distinct communities yield cryptographically unrelated pseudonyms. An attacker who compromises one community's member list learns only pseudonyms meaningless outside that community. |
| I9 | Push relay correlates wake-pushes with content | Wake payload contains only `{type: "wake", ts}` — no content, no record key, no community ID. Relay cannot correlate; platform vendor cannot correlate. The relay daemon does see *which* DHT keys the device cares about, which is metadata; this is opt-in and self-hostable. (See [`../protocol/relay.md`](../protocol/relay.md).) |
| I10 | Game detection leaks "what the user is playing" | Detection is opt-in per identity. Database is bundled (no remote lookup). Output goes only to friends granted presence visibility. Per-friend-group filters ship in the protocol but not yet in the UI. |
| I11 | A coerced user is forced to reveal their pseudonym in another community | Cross-community linkability requires the user to either share the master secret or sign a challenge with two distinct pseudonyms. We provide no API that does this implicitly; OAuth2 / connected-account features that would create this correlation are deliberately omitted. |
| I12 | Address-book matching reveals friend lists to a server | We do not implement address-book matching. Identity is a keypair, not a phone number. There is no server. |

### D — Denial of Service

| ID | Threat | Mitigation |
|----|--------|------------|
| D1 | Spammer floods a community channel with messages | Per-sender rate limit (token bucket, default 10 msg/s). Excess messages are dropped at the gossip stage and not re-broadcast. Governance can adjust the threshold via `AutoModRule`. |
| D2 | Banned member continues writing to their subkey | The CRDT merge drops their entries. The wasted bytes are local-only (their subkey storage, not anyone else's). |
| D3 | Attacker floods the daemon's IPC bus | Per-connection rate limit on `rekindle-node` (100 req/s, refill bucket). Handshake DoS protection (5 s timeout). |
| D4 | Attacker forces excessive DHT writes from a peer | Veilid's own rate limiting at the transport level. Application-level retries use exponential backoff (1 s → 2 s → 4 s → … → 30 s). |
| D5 | Attacker keeps a community's gossip mesh saturated | Gossip TTL=5 caps fan-out depth; dedup cache prevents amplification; per-sender rate limit caps the worst single attacker. |
| D6 | DHT record garbage-collected during low activity | Mutual-aid record warming: idle peers `get_dht_value` subkey 0 of every community record every 5 minutes. Refreshes the network-side TTL without transferring payload. |

### E — Elevation of Privilege

| ID | Threat | Mitigation |
|----|--------|------------|
| E1 | Member without `MANAGE_CHANNELS` creates a channel | Reader-validates: the entry is in the writer's subkey but readers drop it. The writer's view is corrupted only locally. |
| E2 | Member without `BAN_MEMBERS` issues a ban | Same as E1. |
| E3 | Compromised admin issues mass bans / channel deletions | Limited mitigation: an admin with `BAN_MEMBERS` legitimately can ban anyone. Audit log records every governance change with author pseudonym and `lamport`. Other admins can `Unban` if they have higher role position. There is no "undo all" because that would itself be an `ADMINISTRATOR`-class action. |
| E4 | A non-rotator writes a `MEKGenerationBump` to claim authority | Reader-validates rotator (S5). |
| E5 | Daemon escalates from another local user's account | UCred binding in Noise IK prologue: the prologue includes both PID and UID. A different UID dialing the daemon's socket fails the handshake even if it's on the same machine. |

## 4. LINDDUN Privacy Threats

### L — Linkability

| ID | Threat | Mitigation |
|----|--------|------------|
| L1 | Linking the user across communities | Per-community pseudonym derivation; no shared identifier between communities. |
| L2 | Linking the user to their Veilid network identity | Veilid private routes provide receiver anonymity; safety routes provide sender anonymity. Application-level pseudonyms further decouple. |
| L3 | Linking presence updates to a stable identifier | Presence is published to the per-community member registry under the community pseudonym, not the master identity. |
| L4 | Linking a user's relay-routed message to their direct messages | Different Veilid private routes, different envelope formats, different timing. |

### I — Identifiability

| ID | Threat | Mitigation |
|----|--------|------------|
| I-priv1 | A non-friend identifies the user from public DHT records | Profile records are encrypted; only the owner key (held by the user) and friends with explicit access can decrypt the friend list / mailbox / presence. |
| I-priv2 | A community member identifies the user across communities | Pseudonym separation (L1). |
| I-priv3 | A network adversary identifies the user from packet timing | Veilid safety routes obfuscate the sender on every path. The hop count is a fixed 3-hop Tor-class floor (`ANONYMITY_HOP_FLOOR`), not a per-class or user-tunable knob — voice and video get the same anonymity as text, never a lower-hop fast path. |

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

### D — Disclosure (data)

Covered under STRIDE I.

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
| `default.json` limited to window chrome | **Met** — only the `core:window:*` actions the titlebar/drag region use, plus `core:event:allow-listen`/`unlisten` until the per-window Channel router lands |
| No webview creation from the frontend (S8) | **Met** — `core:webview:*` not granted; windows are created only by `src/windows.rs` |
| No `core:default` / plugin `*:default` bundles | **Met** — specific plugin permissions only, in the window that uses them (notification probe: post-auth windows; opener URL: buddy list; dialogs: community) |
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
  (safety routes) help but do not eliminate this. Rekindle fixes the
  hop count at a 3-hop Tor-class floor on every path — including voice
  and video — rather than exposing a lower-hop fast path, accepting the
  latency cost as a deliberate privacy tradeoff.

## 7. How to report a finding

See [`../../SECURITY.md`](../../SECURITY.md). The disclosure channel
is GitHub's private vulnerability flow plus an email backup. We commit
to acknowledging within 3 business days and substantive triage within
10. Findings that align with documented limits in §5 are still welcome
— we may have under-estimated the risk.
