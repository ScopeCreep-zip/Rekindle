# Getting started with Rekindle

This page walks you through installing Rekindle, creating your
identity, and sending your first message. If you only want the
install commands, see [`install.md`](install.md). If you want
walkthroughs of specific features, see [`how-to.md`](how-to.md).

## 1. Install

Rekindle runs on Windows, macOS, and Linux. See
[`install.md`](install.md) for per-platform instructions. The
quickest path is to download a pre-built artifact from the
[Releases page](https://github.com/ScopeCreep-zip/Rekindle/releases)
once the first tagged release is out. Until then, the only path is
to build from source — see
[`../contributor/development.md`](../contributor/development.md).

## 2. Launch and create an identity

When Rekindle launches for the first time, you see the **login
window**. There are no usernames or passwords — your identity is an
Ed25519 keypair. Click **Create new identity** and the app generates
the keypair, opens an `iota_stronghold` vault to hold it, and asks
for a **passphrase**.

The passphrase encrypts the vault on disk. Pick something you'll
remember and that you'd be willing to type a few times a day.
Argon2id is the key-derivation function — slow and memory-hard, so a
medium-strength passphrase is genuinely strong against brute force.
There is **no recovery channel** if you forget it: there is no server
that can reset it for you. Write it down somewhere safe.

You'll be asked for a **display name**. This is the name your
friends see. You can change it later in settings.

## 3. The buddy list

After login you arrive at the **buddy list** — the narrow vertical
window that holds friends, communities, DMs, and your status
controls. The buddy list is the home base of the app. Closing it
hides it to the system tray; closing it via the system tray menu
exits the app.

Right-click the system tray icon to open the **status menu** —
Online / Away / Busy / Offline plus a custom-status field.

## 4. Add your first friend

Friends are added by exchanging Ed25519 public keys (or by following
an invite link). There are three ways:

- **By public key:** click **Add friend** in the buddy list, paste
  the friend's public key, optionally include a message. Your friend
  receives a friend request the next time they connect.
- **By invite link:** ask the friend to send you a `rekindle://`
  invite link. Click it (or paste it into the **Add friend** dialog)
  and Rekindle handles the rest.
- **By QR code:** if the friend is in the same room, scan their QR
  code with your camera (or have them scan yours).

Once accepted, the friend appears in your buddy list. Click them to
open a chat window.

There is no central directory. There is no "find friends from your
contacts." Identity is a keypair, not a phone number — the only way
to add someone is to know their public key out of band.

## 5. Send a message

Click a friend in the buddy list to open a chat window. Type a
message and press **Enter**. The first message between you and a
friend triggers a Signal Protocol X3DH handshake (this happens
automatically and takes a moment); subsequent messages are
end-to-end encrypted with the Double Ratchet. You'll see message
delivery status in the bottom-right of each message bubble: **Sending
→ Sent → Delivered → Read.**

If your friend is offline, the message queues locally and retries
delivery when they come online.

## 6. Join (or create) a community

Communities are Discord-style groups with channels, voice, roles,
permissions, and threads. To **join** an existing community, click
or paste an invite link. To **create** one, click **New community**
in the buddy list, give it a name, and Rekindle generates the
underlying SMPL DHT records on the Veilid network. Share the
generated invite link with whoever should join.

Once you're in a community, the **community window** opens. The
left sidebar shows channels; the right sidebar shows online members.
Each channel is end-to-end encrypted with a per-channel MEK; the
encryption is invisible to you but very visible to anyone trying to
eavesdrop.

## 7. Voice and video

Click a voice channel in any community to join it. The first time
you join, your microphone activates (you can mute with the bottom
panel toggle); audio routes peer-to-peer with sub-50 ms latency for
small groups. For more than 4 participants, one peer in the call
volunteers as a mutual-aid relay so that bandwidth stays manageable
on every member.

For 1:1 calls, click the phone icon in a friend's chat window. The
friend gets a ringing notification and can accept or decline.

## 8. Game detection

If you enable **game detection** in settings, Rekindle scans your
running processes every few seconds and shows your friends what
you're playing. Detection is local-only — Rekindle does not query
any remote service to identify games. You can hide your game
status, choose which friend groups can see it, or disable detection
entirely. See [`how-to.md`](how-to.md) for the controls.

## 9. Cross-device sync

If you want Rekindle on a second device (laptop + desktop, for
example), open settings on the existing device and click **Pair
another device**. You'll see a one-time code and a QR code. On the
new device, choose **Pair with existing identity** and either scan
the QR or type the code + salt. The new device joins your identity
and stays in sync (friend list, communities, read state,
preferences) over the personal sync record.

## What's next

- [`how-to.md`](how-to.md) — walkthroughs for friend groups, voice
  calls, file sharing, presence visibility, etc.
- [`faq.md`](faq.md) — common questions and project quirks.
- [`../../ARCHITECTURE.md`](../../ARCHITECTURE.md) — if you want to
  understand how the app works under the hood.
- [`../../SECURITY.md`](../../SECURITY.md) — if you found a
  vulnerability and want to report it privately.
