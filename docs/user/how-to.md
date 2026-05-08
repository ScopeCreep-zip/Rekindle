# How-to walkthroughs

Step-by-step recipes for common tasks. Each section is self-contained
and assumes you have Rekindle installed and an identity created (see
[`getting-started.md`](getting-started.md)).

## Add a friend

### By public key

1. Get your friend's Ed25519 public key (from their **Profile → Copy
   public key**, or however they shared it).
2. In your buddy list, click **Add friend**.
3. Paste the public key. Optionally write a short message (it
   accompanies the friend request).
4. Click **Send**. Your friend receives the request next time they
   come online.

### By invite link

1. Ask your friend to generate an invite link (**Profile → Generate
   invite link**).
2. Click the `rekindle://...` link in your browser, or paste it
   into the **Add friend** dialog.
3. Rekindle decrypts the invite and proposes adding the sender as a
   friend. Confirm.

### By QR code

1. Friend selects **Profile → Show QR code**.
2. You select **Add friend → Scan QR**, point your device's camera
   at the screen.
3. Confirm.

### Verify a friend's identity (recommended)

After adding a friend, verify their identity-key fingerprint
out-of-band (in person, on a video call you both trust, etc.).

1. Open the friend's profile in Rekindle.
2. Both of you see the same **safety code** (numeric or QR).
3. Compare. If they match, click **Mark verified**. The friend gets
   a green check next to their name.
4. If the safety code ever changes (re-installation, key
   compromise), Rekindle will surface a re-verification prompt; do
   not click "trust" without re-verifying.

## Create a community

1. In the buddy list, click **New community**.
2. Enter a name and optional description.
3. Choose **Create**. Rekindle generates the underlying Veilid SMPL
   records and opens the community window.
4. The community starts with a `#general` text channel and a
   `#voice` voice channel; you can add more from the channel
   sidebar.
5. Click **Settings → Invites → Generate invite link** to invite
   members.

## Join a community

1. Click the `rekindle://invite/...` link your friend sent.
2. Rekindle decrypts the invite, shows the community name, and asks
   to confirm.
3. Confirm. Rekindle goes through the self-sovereign join flow:
   bootstraps governance state, claims an empty member slot,
   subscribes to channels.
4. After ~1–2 seconds, the community window opens.

## Manage friend groups (privacy)

Friend groups partition your friend list into separate visibility
contexts. By default everyone is in the **default** group; you can
create more (e.g. "gaming friends", "work friends", "family") and
control:

- **Presence visibility:** Each friend group can be marked
  *limit visibility*, in which case members of that group don't
  receive your full presence updates (no game info, no custom
  status).
- **Game detection visibility:** As a sub-control of presence —
  hide game info from specific friend groups.

### Create a group

1. **Buddy list → Manage groups**.
2. Click **New group**, enter a name.
3. Optionally toggle **Limit visibility**.

### Move a friend between groups

Drag the friend's buddy list entry into the target group, or
right-click → **Move to group → ...**

## Send files (Lost Cargo)

1. In any chat or community channel, drag a file into the message
   input (or use the paperclip icon).
2. Rekindle chunks the file (28 KB chunks), encrypts each chunk with
   a per-file FEK, and announces the attachment via an
   `AttachmentOffer`.
3. Recipients see the attachment immediately (notification + preview
   for images). They download chunks from whoever in the community
   has them — including you, while you're online.
4. Once even one recipient has the full file, you can go offline
   and the file remains available to other members.

### Pin a file

If a file is important and shouldn't be evicted from member caches,
admins can **pin** it (right-click → **Pin attachment**). Pinned
files are exempt from the LRU eviction in every member's cache.

## Start a voice call

### 1:1 voice call

1. Open the friend's chat window.
2. Click the **phone** icon in the titlebar.
3. The friend gets a ringing notification. They can accept or
   decline. Once accepted, audio flows peer-to-peer with sub-50 ms
   latency.

### Community voice channel

1. Click any voice channel in the community sidebar.
2. You're connected. Other members in the channel hear you; you
   hear them.
3. Bottom panel toggles: **Mute** (toggle microphone),
   **Deafen** (toggle speaker), **Server-mute** (admin: silence
   another participant).

### Video / screen share

In a voice channel, click **Start video** or **Share screen**. Video
quality is currently ~480p @ 15 fps; this will improve with future
work on `veilid-media` upstream.

## Pair another device

1. On the existing device, **Settings → Devices → Pair another
   device**.
2. A one-time code, salt, and QR appear. The QR encodes
   `code ‖ salt ‖ personal_record_key`.
3. On the new device, choose **Pair with existing identity** and
   either scan the QR or type the code + salt.
4. The new device dials back via Veilid `app_call`, the existing
   device wraps the master secret with a one-time key derived from
   the code + salt, and ships it.
5. The new device unwraps, persists locally, and the device list
   updates everywhere.

The pairing code is single-use and expires after 5 minutes. If
something goes wrong, generate a new one.

## Manage notifications

### Per-channel mute

Right-click a channel → **Notification level**:

- **All messages** — every message generates a notification.
- **Mentions only** — only when you or a role you hold is mentioned.
- **None** — no notifications from this channel.

### Community-wide default

**Community settings → Notifications → Default level**.

### Do Not Disturb

Click your status indicator → **Do Not Disturb** to silence
notifications app-wide. Quiet hours can also be scheduled in
**Settings → Notifications → Quiet hours**.

### Mobile push (when enabled)

If you're on a mobile target with **Push relay** enabled, you can
choose Tier 2 (background fetch only) or Tier 3 (opt-in opaque
wake-push via FCM/APNs). See
[`../protocol/relay.md`](../protocol/relay.md) for the privacy
properties.

## Customize your appearance

- **Display name:** Settings → Profile → Display name
- **Avatar:** Settings → Profile → Avatar (drop an image; it's
  resized and stored locally)
- **Banner / pronouns / bio:** Settings → Profile (community-
  visible only — different from your global identity)

Within each community you can also set a **per-community profile**
(distinct display name, avatar, bio) that applies only inside that
community.

## Revoke a friend

Right-click a friend → **Remove friend**. They're moved out of your
buddy list immediately. Past chat history with them remains in your
local SQLite (deletion of the record is destructive — you can't
get it back). MEK rotation does not apply to 1:1 chats; the next
session establishment with this friend (if you re-friend) starts
fresh via X3DH.

## Block a user

Right-click a peer → **Block**. Blocked peers cannot send you
messages; their friend requests are auto-rejected; their gossip
envelopes from communities you share are dropped at the receive
side.

## Leave a community

Open the community → **Settings → Leave community**. Rekindle stops
your presence heartbeat, zeroes your registry slot (so the slot
becomes available for future joiners), closes all DHT records, and
removes the community from your list. Optionally also delete local
SQLite data for the community.

## Export your data

There is no central export server because there is no central data
store — your data is already local.

- Local SQLite is at the path listed in
  [`install.md`](install.md). You can copy `db.sqlite3` and your
  Stronghold vault to back up your identity and history.
- Cap'n Proto schemas at `schemas/` document the wire format if
  you want to write a custom export tool.
