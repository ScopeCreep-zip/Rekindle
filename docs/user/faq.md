# Frequently Asked Questions

## General

### What is Rekindle?

Rekindle is a 1:1 reimplementation of the classic [Xfire](https://en.wikipedia.org/wiki/Xfire)
gaming chat client, rebuilt as a modern decentralised peer-to-peer
desktop application. It provides end-to-end encrypted 1:1 messaging,
direct messages, communities with channels and voice / video, voice
calling, cross-device sync, and cross-platform game detection — all
without any central server.

### Is it free?

Yes. Rekindle is MIT-licensed open source. There are no premium
tiers, subscriptions, ads, telemetry, or in-app purchases. There are
no plans to add any.

### Why "Rekindle"?

Xfire was an icon of mid-2000s gaming chat. The name is a tribute and
a statement: rekindling that experience on a substrate that respects
the user.

### Who is it for?

Anyone who wants:

- A chat app with no central server, no telemetry, and no platform-
  vendor account.
- Strong end-to-end encryption with forward secrecy.
- Game detection and rich presence — see what your friends are
  playing.
- The classic Xfire visual identity.

It's particularly suited to gaming friend groups, privacy-conscious
users, activists, journalists, and anyone who would be materially
harmed by metadata collection or content disclosure.

### Is it ready to use?

Rekindle is **pre-1.0**. 1:1 messaging, voice, friends, communities,
DMs, game detection, and cross-device sync are substantially
complete. We are actively migrating the community layer to a v2.0
"chiral network" architecture. Behaviour, file formats, and protocols
may change between updates. See [`../roadmap.md`](../roadmap.md).

## Privacy and security

### Where is my data stored?

Locally. Your identity vault is encrypted on your disk with a key
derived from your passphrase via Argon2id. Your messages, friends,
communities, and preferences live in a SQLite database also on your
disk. Nothing about you is stored anywhere else.

DHT records — your published profile, prekeys, presence, community
membership — are stored on the Veilid network, but encrypted at the
application layer before they're written. Veilid relay nodes see only
ciphertext.

### Can the developers read my messages?

No. There is no developer-controlled server. Messages are end-to-end
encrypted (Signal Protocol for 1:1, MEK for community channels)
between the participants only. We have no key, no relay, and no log
that could read them.

### Can the government subpoena my chats?

There is no Rekindle company to subpoena. There is no server to
serve. The only place your data exists is on the devices of you and
your conversation partners. A subpoena to one of those devices
forces the device's owner to comply (or not, depending on
jurisdiction); the protocol cannot help one user against another's
direct compliance.

### Is it actually anonymous?

Veilid provides Tor-style sender + receiver anonymity at the
network layer. Rekindle adds per-community pseudonyms that are
unlinkable to each other. The combination is strong but not
absolute — see [`../security/privacy-properties.md`](../security/privacy-properties.md)
for what is and is not protected.

### What if I lose my device?

If your device is lost without first pairing a backup device:

- Your identity vault is on the lost device. Without the passphrase,
  it is not accessible to whoever has the device.
- Without your master secret, you cannot decrypt history or rejoin
  communities. There is **no recovery channel** — neither we nor
  any service can reset your account.
- Your friends and communities still exist; you can create a new
  identity, share its public key with friends, and rejoin
  communities via fresh invites.

To prevent this, **pair a second device** before losing access to
your first. The pairing flow transfers the master secret in a
key-agreement handshake.

### What if my passphrase is compromised?

If you suspect your passphrase has been observed:

1. Pair a new device that you control.
2. On the new device, change the passphrase.
3. Wipe the old device's app data.

Master-secret rotation is on the roadmap but not yet shipped — if
the master secret itself was extracted, the only mitigation today is
to abandon the identity and create a new one.

## Features and limitations

### Why are some Discord features missing?

Several Discord features are **deliberately omitted** because they
conflict with the privacy posture or the P2P architecture. Examples:
webhooks, server-hosted bots, OAuth2 connected accounts, server boost
/ premium tiers, server discovery directory, phone / email
verification, server-side audit logs, vanity URLs, Activities,
Clyde AI. The full list with rationale is in
[`../architecture/communities.md`](../architecture/communities.md) §13.

### Why is the buddy list so narrow?

Visual fidelity to the classic Xfire client is an explicit project
goal. The narrow vertical buddy list (320 px wide) is part of that
identity.

### Where are typing indicators?

Implemented. They use the gossip mesh (ephemeral, no DHT write).

### Why does video look low-resolution?

Veilid's `app_message` primitive caps payloads at ~32 KB. Video
frames must be chunked to fit, with FEC for loss tolerance. We
currently target ~480p at 15 fps with ~800 kbps. Higher quality
needs upstream work on `veilid-media`. See
[`../architecture/voice.md`](../architecture/voice.md) for context.

### Why isn't message X / file Y / message Z showing up?

Several possibilities:

- **The sender is offline and the gossip path didn't reach you.**
  The DHT path will catch up within ~60 seconds (the
  `inspect_dht_record` poll cycle).
- **The community has more than 255 members and you're in a
  different segment.** Cross-segment offline catch-up is on the
  roadmap (Plate Gates C1-2). Online conversations work
  cross-segment via gossip.
- **The file's chunks aren't held by any online peer.** Lost Cargo
  needs at least one online peer with the chunks. Pinning files
  improves this.

### Why does my mobile push notification not show content?

Mobile push relay sends a content-free wake notification. Apple and
Google can see the wake but cannot see the message. The app fetches
the actual content over Veilid after the OS wakes it. This is a
deliberate privacy choice — see
[`../protocol/relay.md`](../protocol/relay.md).

### Why is there no spam filter?

Server-side content filtering is impossible with end-to-end
encryption — there is no server that can read messages. Rekindle
relies on:

1. **Per-sender gossip rate limit** (default 10 messages/sec).
2. **Governance moderation** — community admins can ban; the ban
   propagates via gossip and reader-validates.
3. **Client-side keyword/regex filters** (in development).

Communities self-govern. There is no platform appeal layer.

### Why doesn't message deletion really delete?

Deletion in P2P is fundamentally advisory. A `Delete` tombstone
asks every peer to remove the message from local storage and UI.
Honest clients comply. A modified client could retain the message.
SMPL subkeys can be overwritten but the old value may be cached by
relay nodes. This is inherent to E2E encrypted P2P.

## Technical

### What is Veilid?

Veilid is the peer-to-peer network that Rekindle uses for transport.
It provides anonymous routing, DHT storage, NAT traversal, and
hop-by-hop encryption. Open source under Apache 2.0; see
[`../decisions/0001-veilid-as-transport.md`](../decisions/0001-veilid-as-transport.md)
for why we chose it.

### What is the Signal Protocol?

The Signal Protocol provides end-to-end encryption with forward
secrecy for 1:1 messaging. It's the protocol used by Signal,
WhatsApp, Facebook Messenger's secret chats, Skype's private
conversations, and others. We use it for friend-to-friend messaging.
See [`../decisions/0002-signal-protocol-for-1to1.md`](../decisions/0002-signal-protocol-for-1to1.md).

### How big can a community be?

A single Veilid SMPL DHT record holds 255 member subkeys. Larger
communities are split into segments via the **Plate Gate**
mechanism — a 1000-member community uses 4 segments. The hard cap is
8 segments (~2040 members) today; raising the cap is a
constant-tweak. Discord-scale (100,000+ members) is architecturally
incompatible with full-mesh gossip and is a deliberate boundary.

### What runs in the background when I'm not using the app?

When the app is open, a Veilid node is running in the desktop process
or in `rekindle-node` daemon. The node:

- Maintains private routes (refreshed every ~120 s).
- Serves gossip relays for communities you're in.
- Watches DHT records you care about.
- Periodically warms records (every 5 minutes — touches subkey 0 to
  refresh the network-side TTL).
- Sends presence heartbeats every 15 s in communities you're in.

If you close the app fully, none of this happens. Mobile platforms
have a separate three-tier escalation for notifications.

### Why is the desktop app and the CLI separate?

The desktop app embeds Veilid in-process today. The CLI is a client
of a separate daemon (`rekindle-node`) that owns Veilid and serves
multiple frontends over an encrypted IPC bus. Both speak the same
protocol. Eventually the desktop app may migrate to the daemon
model. See [`../decisions/0005-daemon-cli-track.md`](../decisions/0005-daemon-cli-track.md).

### Can I run a "server"?

There are no servers in the protocol. There are two roles a user
can run:

- **Headless community member.** Run `rekindle-node` (the daemon)
  on a server you control with no GUI. The daemon participates in
  communities exactly like any other member — the only difference
  is it has no eyes attached. Useful for keeping a community's
  records warm and gossip mesh active.
- **Push relay (`rekindle-push-relay`).** A self-hostable
  headless `veilid-server` that watches DHT records on behalf of
  registered mobile devices and sends content-free FCM/APNs wake
  notifications.

Neither role is privileged. Both are members of the network like
anyone else.

## Reporting issues

### How do I file a bug?

Open an issue at
<https://github.com/ScopeCreep-zip/Rekindle/issues> using the **Bug
report** template. Don't include private keys, message content, or
session data.

### How do I report a security vulnerability?

**Don't file a public issue.** Use the private channel described in
[`../../SECURITY.md`](../../SECURITY.md).

### Where do I ask a question?

If it's about how to use Rekindle, this FAQ first, then
[`how-to.md`](how-to.md), then a GitHub Discussion (when one is
opened — pre-1.0 they're handled in issues).

If it's about how the protocol works, see
[`../architecture/`](../architecture/) and
[`../protocol/`](../protocol/).

### How do I contribute?

See [`../../CONTRIBUTING.md`](../../CONTRIBUTING.md).
