-- Rekindle initial schema migration
-- Creates all core tables for identity, friends, messaging, communities, and Signal protocol state.

CREATE TABLE IF NOT EXISTS identity (
    id INTEGER PRIMARY KEY,
    public_key TEXT NOT NULL UNIQUE,
    display_name TEXT,
    created_at INTEGER NOT NULL,
    -- The owned records' keys. Their owner keypairs are in the vault
    -- (`rekindle_vault::typed::owner_keys`, plan D1).
    dht_record_key TEXT,
    friend_list_dht_key TEXT,
    avatar_webp BLOB,
    account_dht_key TEXT,
    mailbox_dht_key TEXT,
    -- Architecture §28.4: personal DFLT record key for cross-device sync.
    -- Created lazily on first launch when the user opts into multi-device.
    personal_sync_record_key TEXT,
    -- This device's id: 16 random bytes, lowercase hex, minted once by the
    -- own-records stage (`repo::identity::ensure_device_id`); the prekey
    -- device list and the personal sync record name it, neither replaces it.
    device_id TEXT CHECK(device_id IS NULL
        OR (length(device_id) = 32 AND device_id NOT GLOB '*[^0-9a-f]*')),
    -- This device's prekey record (plan E2.4, DFLT(101)); its owner
    -- keypair is in the vault (`OwnerKey::Prekeys`).
    prekey_record_key TEXT,
    -- The friend-request token our friend code carries (plan E2.5, D17):
    -- 16 random bytes, unpadded lowercase base32, minted with the identity
    -- (`rekindle_secrets::friend_token::mint`); rotating it refuses requests
    -- sent with the old code.
    friend_request_token TEXT NOT NULL CHECK(length(friend_request_token) = 26
        AND friend_request_token NOT GLOB '*[^a-z2-7]*')
);

CREATE TABLE IF NOT EXISTS friend_groups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    UNIQUE(owner_key, name)
);

CREATE TABLE IF NOT EXISTS friends (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    public_key TEXT NOT NULL,
    display_name TEXT,
    nickname TEXT,
    group_id INTEGER REFERENCES friend_groups(id) ON DELETE SET NULL,
    added_at INTEGER NOT NULL,
    dht_record_key TEXT,
    last_seen_at INTEGER,
    avatar_webp BLOB,
    mailbox_dht_key TEXT,
    -- `FriendshipState` (plan D2). `removing`: we removed them and the
    -- `Unfriended` notice is still queued; the row keeps their routing
    -- information until nothing is queued to them.
    friendship_state TEXT NOT NULL
        CHECK(friendship_state IN ('pending_out', 'accepted', 'removing')),
    -- Phase 2 Track A.2 — Friend's most-recently-seen DeviceId
    -- (hex Ed25519 device pubkey). NULL until they advertise via the
    -- DeviceList owner subkey on their per-pair inbox.
    current_device_id TEXT,
    -- Plan E2.5: the friend's current friend-presence key (their
    -- `FriendsKey`), which opens their profile's STATUS and GAME_INFO.
    -- Cleared when we remove them; the row goes when they or we block.
    presence_epoch INTEGER CHECK(presence_epoch > 0),
    presence_key BLOB CHECK(length(presence_key) = 32),
    CHECK((presence_epoch IS NULL) = (presence_key IS NULL)),
    PRIMARY KEY (owner_key, public_key)
);

-- Plan E2.5: our friend-presence key, the newest epoch only (an older one
-- seals nothing more). Profile subkeys 2 and 4 are sealed under it and it
-- is sent to each accepted friend as a ratcheted `FriendsKey`; a new epoch
-- is minted on every unfriend or block.
CREATE TABLE IF NOT EXISTS presence_keys (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    epoch INTEGER NOT NULL CHECK(epoch > 0),
    key BLOB NOT NULL CHECK(length(key) = 32),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, epoch)
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    conversation_type TEXT NOT NULL CHECK(conversation_type IN ('dm', 'channel')),
    sender_key TEXT NOT NULL,
    body TEXT NOT NULL,
    automod_blurred INTEGER NOT NULL DEFAULT 0,
    timestamp INTEGER NOT NULL,
    is_read INTEGER NOT NULL DEFAULT 0,
    reply_to_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    attachment_json TEXT CHECK(json_valid(attachment_json)),
    mek_generation INTEGER,
    lamport_ts INTEGER DEFAULT 0,
    message_id TEXT,
    forwarded_from_author TEXT,
    flags INTEGER NOT NULL DEFAULT 0,
    -- D22's delivery vocabulary (plan D1, C7.14; `SendState`). Our own
    -- channel post is written `queued`, then `sent` when its record slot
    -- settles with it, or `failed`; a received message is `delivered`.
    send_state TEXT NOT NULL DEFAULT 'delivered'
        CHECK(send_state IN ('queued', 'sent', 'delivered', 'read', 'failed')),
    -- While `queued` on a held write: the record slot it waits on, so the
    -- next login re-registers it when the write is restored (C7.6h).
    held_record_key TEXT,
    held_subkey INTEGER
);

CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(owner_key, conversation_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_unread ON messages(owner_key, conversation_id, is_read) WHERE is_read = 0;
-- A row with no client id (a channel message from a path that carries
-- none) is deduplicated by its sender and time; a row with one by its
-- author's client id, so two messages in one millisecond stay two, and a
-- peer cannot reuse our id to suppress or overwrite our row.
CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_dedup
  ON messages(owner_key, conversation_id, conversation_type, sender_key, timestamp)
  WHERE message_id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_message_id
  ON messages(owner_key, conversation_id, sender_key, message_id) WHERE message_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS communities (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    icon_hash TEXT,
    banner_hash TEXT,
    my_role_ids TEXT NOT NULL DEFAULT '[0]' CHECK(json_valid(my_role_ids)),
    joined_at INTEGER NOT NULL,
    last_synced INTEGER,
    dht_record_key TEXT,
    dht_owner_keypair TEXT,
    my_pseudonym_key TEXT,
    mek_generation INTEGER NOT NULL DEFAULT 0,
    manifest_key TEXT,
    member_registry_key TEXT,
    my_subkey_index INTEGER,
    -- Plate Gate: which segment the user joined into. 0 = primary.
    my_segment_index INTEGER NOT NULL DEFAULT 0,
    governance_key TEXT,
    lamport_clock INTEGER NOT NULL DEFAULT 0,
    -- Local-only presence sharing consent (default-deny). JSON-encoded
    -- PresenceSharingPolicy; NULL falls back to the default-deny policy.
    -- Never published to the registry — this is the user's private choice.
    presence_policy TEXT CHECK(json_valid(presence_policy)),
    PRIMARY KEY (owner_key, id)
);

CREATE TABLE IF NOT EXISTS channels (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    id TEXT NOT NULL,
    community_id TEXT NOT NULL,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL CHECK(channel_type IN ('text', 'voice', 'announcement', 'forum', 'stage', 'directory', 'media', 'events', 'dm')),
    sort_order INTEGER NOT NULL DEFAULT 0,
    category_id TEXT,
    topic TEXT NOT NULL DEFAULT '',
    slowmode_seconds INTEGER NOT NULL DEFAULT 0,
    nsfw INTEGER NOT NULL DEFAULT 0,
    message_record_key TEXT,
    mek_generation INTEGER NOT NULL DEFAULT 0,
    log_key TEXT,
    my_sequence INTEGER NOT NULL DEFAULT 0,
    -- Architecture §10.8 — text-in-voice. Hex channel id of the parent
    -- voice channel when this channel is the text companion of a voice
    -- channel; NULL for normal channels. Frontend uses this to hide
    -- text-in-voice channels unless the local member is connected to
    -- the parent voice.
    parent_voice_channel_id TEXT,
    PRIMARY KEY (owner_key, id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS community_members (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    community_id TEXT NOT NULL,
    pseudonym_key TEXT NOT NULL,
    display_name TEXT,
    role_ids TEXT NOT NULL DEFAULT '[0,1]' CHECK(json_valid(role_ids)),
    timeout_until INTEGER,
    joined_at INTEGER NOT NULL,
    -- LOCAL slot within the segment's registry record (raw SMPL subkey
    -- 0..254 — records are o_cnt:0, no owner offset). Written by the
    -- registry scan and the JoinAccepted upsert; NULL until first
    -- discovered.
    subkey_index INTEGER,
    -- Plate Gate (architecture §15): which segment hosts this member's slot.
    -- 0 = primary segment (the genesis registry); 1..=MAX_SEGMENTS for each
    -- expansion. The global slot (keypair derivation) is
    -- `segment_index * 255 + subkey_index`.
    segment_index INTEGER NOT NULL DEFAULT 0,
    onboarding_complete INTEGER NOT NULL DEFAULT 0,
    -- Per-community profile fields (architecture §8.2 / §24.2). Mirrored
    -- from peer MemberPresence at registry-poll time so they survive
    -- restarts; without these the member list shows pseudonym keys
    -- after a relaunch.
    bio TEXT,
    pronouns TEXT,
    theme_color INTEGER,
    badges TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(badges)),
    -- BLAKE3 content references for avatar + banner. The bytes
    -- themselves live in the local Lost Cargo cache or as an inline
    -- expression asset (architecture §32 Week 15).
    avatar_ref TEXT,
    banner_ref TEXT,
    PRIMARY KEY (owner_key, community_id, pseudonym_key),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS community_roles (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    role_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    color INTEGER NOT NULL DEFAULT 0,
    permissions INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL DEFAULT 0,
    hoist INTEGER NOT NULL DEFAULT 0,
    mentionable INTEGER NOT NULL DEFAULT 0,
    self_assignable INTEGER NOT NULL DEFAULT 0,
    -- Architecture §19.4 — mutually exclusive role groups (NULL = independent).
    exclusion_group TEXT,
    PRIMARY KEY (owner_key, community_id, role_id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS community_categories (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, community_id, id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Per-channel permission overwrites (role or member specific allow/deny).
CREATE TABLE IF NOT EXISTS channel_overwrites (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK(target_type IN ('role', 'member')),
    target_id TEXT NOT NULL,
    allow INTEGER NOT NULL DEFAULT 0,
    deny INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, community_id, channel_id, target_type, target_id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Performance indexes for JOIN keys
CREATE INDEX IF NOT EXISTS idx_friends_group_id ON friends(owner_key, group_id);
CREATE INDEX IF NOT EXISTS idx_channels_community_id ON channels(owner_key, community_id);
CREATE INDEX IF NOT EXISTS idx_community_members_community ON community_members(owner_key, community_id, pseudonym_key);

CREATE TABLE IF NOT EXISTS pending_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    recipient_key TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_pending_recipient ON pending_messages (owner_key, recipient_key);

-- Phase 4 — tamper-evident audit chain. One row per audit-worthy action
-- (friend add/remove, channel join/leave, identity rotate, vault unlock).
-- `mac = BLAKE3-keyed(audit_mac_key, prev_mac || cursor_le || payload_json)`.
-- Verified by the `audit_verify` Tauri command and on vault unlock; a
-- broken chain emits `SystemEvent::AuditChainBroken` and a typed
-- `notification-event` toast.
CREATE TABLE IF NOT EXISTS audit_entries (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    cursor INTEGER NOT NULL,
    prev_mac BLOB NOT NULL,
    mac BLOB NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    PRIMARY KEY (owner_key, cursor)
);

CREATE TABLE IF NOT EXISTS pending_friend_requests (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    public_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    message TEXT NOT NULL DEFAULT '',
    received_at INTEGER NOT NULL,
    profile_dht_key TEXT,
    route_blob BLOB,
    mailbox_dht_key TEXT,
    invite_id TEXT,
    PRIMARY KEY (owner_key, public_key)
);

-- Plan §Failure 5 / Architecture §10.10 — direct call ring timeouts.
-- Written when an Outgoing call's 30 s timer fires without an accept,
-- or when an Incoming call expires without the local user answering.
-- Surfaced via `getMissedCalls` for the BuddyList badge.
CREATE TABLE IF NOT EXISTS missed_calls (
    call_id TEXT NOT NULL,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer_key TEXT NOT NULL,
    kind INTEGER NOT NULL CHECK(kind IN (0, 1)),
    expired_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, call_id)
);

CREATE INDEX IF NOT EXISTS idx_missed_calls_owner_peer
    ON missed_calls(owner_key, peer_key, expired_at DESC);

CREATE TABLE IF NOT EXISTS blocked_users (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    public_key TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    blocked_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, public_key)
);

-- Per-channel/community notification preferences (overrides global settings).
-- When channel_id is NULL, the preference applies to the whole community.
-- level: 0 = AllMessages, 1 = MentionsOnly, 2 = None (muted)
CREATE TABLE IF NOT EXISTS notification_preferences (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL DEFAULT '',
    level INTEGER NOT NULL DEFAULT 0
        CHECK(level IN (0, 1, 2)),
    -- Architecture §32 Phase 7 Week 25 — per-community + per-channel
    -- notification sound override. NULL means "inherit from the next
    -- level up": channel row falls through to the (community_id, '')
    -- row, which falls through to the app-global `notification_sound`
    -- bool in `app_settings`. Value is an expression `sound_id` from
    -- the community soundboard (`ExpressionAdded { kind: "sound" }`).
    sound_ref TEXT,
    PRIMARY KEY (owner_key, community_id, channel_id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS app_settings (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    quiet_hours_enabled INTEGER NOT NULL DEFAULT 0,
    quiet_hours_start_minute INTEGER NOT NULL DEFAULT 1320
        CHECK(quiet_hours_start_minute >= 0 AND quiet_hours_start_minute < 1440),
    quiet_hours_end_minute INTEGER NOT NULL DEFAULT 420
        CHECK(quiet_hours_end_minute >= 0 AND quiet_hours_end_minute < 1440),
    -- Architecture §17.2 — IANA timezone string (e.g.,
    -- "America/Los_Angeles") drives DST-aware quiet-hours resolution.
    -- The frontend defaults to `Intl.DateTimeFormat().resolvedOptions().timeZone`
    -- on first save; "UTC" is the safe seed when no row exists.
    quiet_hours_timezone TEXT NOT NULL DEFAULT 'UTC',
    -- Architecture §32 Phase 7 Week 25 — global Do Not Disturb. When 1,
    -- ALL notification dispatches are suppressed regardless of channel
    -- level, community default, quiet hours window, or mention status.
    -- The user-facing UI surfaces it as a one-click toggle in the tray
    -- menu separate from the time-bounded quiet hours.
    do_not_disturb INTEGER NOT NULL DEFAULT 0
        CHECK(do_not_disturb IN (0, 1)),
    -- Architecture §28.8 line 3220 — link preview generation reveals the
    -- user's IP to the target server (the OpenGraph fetch bypasses
    -- Veilid). When 0, the sender skips the fetch entirely so URLs
    -- remain bare in the chat. Default 0: the IP leak is opt-in, and a row
    -- created by another setting (do-not-disturb, quiet hours) must not
    -- switch previews on (plan A1.7, C7.28).
    link_previews_enabled INTEGER NOT NULL DEFAULT 0
        CHECK(link_previews_enabled IN (0, 1)),
    -- The user's preferences (plan D1; `rekindle_types::preferences`):
    -- one column per field, defaults matching `Preferences::default`.
    notifications_enabled INTEGER NOT NULL DEFAULT 1 CHECK(notifications_enabled IN (0, 1)),
    notification_sound INTEGER NOT NULL DEFAULT 1 CHECK(notification_sound IN (0, 1)),
    start_minimized INTEGER NOT NULL DEFAULT 0 CHECK(start_minimized IN (0, 1)),
    game_detection_enabled INTEGER NOT NULL DEFAULT 1 CHECK(game_detection_enabled IN (0, 1)),
    game_scan_interval_secs INTEGER NOT NULL DEFAULT 15
        CHECK(game_scan_interval_secs BETWEEN 1 AND 3600),
    input_device TEXT,
    output_device TEXT,
    video_device_id TEXT,
    video_device_label TEXT,
    input_volume REAL NOT NULL DEFAULT 1.0 CHECK(input_volume BETWEEN 0.0 AND 1.0),
    output_volume REAL NOT NULL DEFAULT 1.0 CHECK(output_volume BETWEEN 0.0 AND 1.0),
    noise_suppression INTEGER NOT NULL DEFAULT 1 CHECK(noise_suppression IN (0, 1)),
    echo_cancellation INTEGER NOT NULL DEFAULT 1 CHECK(echo_cancellation IN (0, 1)),
    auto_away_minutes INTEGER NOT NULL DEFAULT 10 CHECK(auto_away_minutes BETWEEN 0 AND 1440),
    auto_volunteer_relay_for_new_friends INTEGER NOT NULL DEFAULT 0
        CHECK(auto_volunteer_relay_for_new_friends IN (0, 1)),
    ringtone_enabled INTEGER NOT NULL DEFAULT 1 CHECK(ringtone_enabled IN (0, 1)),
    ringtone_volume REAL NOT NULL DEFAULT 0.4 CHECK(ringtone_volume BETWEEN 0.0 AND 1.0),
    in_call_dnd_auto_enable INTEGER NOT NULL DEFAULT 1 CHECK(in_call_dnd_auto_enable IN (0, 1)),
    PRIMARY KEY (owner_key)
);

-- The input-channel choice per input device (`Preferences::input_channels`,
-- plan C7.24b): a device without rows averages all its channels.
CREATE TABLE IF NOT EXISTS app_input_channels (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    device_name TEXT NOT NULL,
    channel INTEGER NOT NULL CHECK(channel BETWEEN 0 AND 65535),
    PRIMARY KEY (owner_key, device_name, channel)
);

-- Architecture §28.7 / §32 W18 — per-channel slowmode bookkeeping.
-- Tracks the millisecond timestamp of this device's last send into a
-- channel so the cooldown window survives app restarts. Without this
-- table the in-memory `channel_last_send_at` resets on relaunch and a
-- user could circumvent a channel's configured slowmode by quitting.
CREATE TABLE IF NOT EXISTS channel_slowmode_state (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    last_send_ms INTEGER NOT NULL,
    PRIMARY KEY (owner_key, community_id, channel_id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_channel_slowmode_state_owner
    ON channel_slowmode_state(owner_key, community_id);

CREATE TABLE IF NOT EXISTS outgoing_invites (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    invite_id TEXT NOT NULL,
    url TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK(status IN ('pending', 'responded', 'accepted', 'rejected', 'cancelled', 'expired')),
    accepted_by TEXT,
    PRIMARY KEY (owner_key, invite_id)
);

-- Community threads (forum-style sub-conversations within a channel).
CREATE TABLE IF NOT EXISTS community_threads (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    name TEXT NOT NULL,
    starter_message_id TEXT NOT NULL,
    creator_pseudonym TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    auto_archive_seconds INTEGER NOT NULL DEFAULT 0,
    last_message_at INTEGER NOT NULL DEFAULT 0,
    message_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, community_id, id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Thread messages (messages within a thread).
-- Thread reply messages (architecture §11). The synthetic `id` PK is
-- required by FTS5 (`content_rowid` must be a single INTEGER column),
-- with the original logical key kept as a UNIQUE constraint.
CREATE TABLE IF NOT EXISTS thread_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    sender_pseudonym TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    timestamp INTEGER NOT NULL,
    reply_to_id TEXT,
    UNIQUE (owner_key, community_id, thread_id, message_id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_thread_messages_thread
    ON thread_messages(owner_key, community_id, thread_id, timestamp);

-- Community events (scheduled events with RSVPs).
CREATE TABLE IF NOT EXISTS community_events (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    creator_pseudonym TEXT NOT NULL,
    start_time INTEGER NOT NULL,
    end_time INTEGER,
    channel_id TEXT,
    max_attendees INTEGER,
    created_at INTEGER NOT NULL,
    -- Architecture §21 line 2630 — Scheduled / Active / Completed /
    -- Cancelled. CHECK constraint matches the EventStatus enum
    -- (`rekindle-types::event::EventStatus`).
    status TEXT NOT NULL DEFAULT 'scheduled'
        CHECK(status IN ('scheduled', 'active', 'completed', 'cancelled')),
    -- Architecture §21 line 2624 — peer-cached cover image hex hash.
    cover_image_ref TEXT,
    -- Architecture §21 line 2628 — JSON-serialized RecurrenceRule.
    -- NULL for one-off events; non-NULL holds the full struct so the
    -- frontend can render "every 2 weeks on Mon, Wed" without back-end
    -- date math.
    recurrence_json TEXT CHECK(json_valid(recurrence_json)),
    -- Architecture §21 line 2629 — JSON-serialized EventLocation enum.
    -- Falls back to channel_id-based VoiceChannel for legacy rows.
    location_json TEXT CHECK(json_valid(location_json)),
    PRIMARY KEY (owner_key, community_id, id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Event RSVPs (one per member per event).
CREATE TABLE IF NOT EXISTS event_rsvps (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    pseudonym_key TEXT NOT NULL,
    -- `RsvpStatus` (rekindle-types::event).
    status TEXT NOT NULL CHECK(status IN ('going', 'interested', 'declined')),
    PRIMARY KEY (owner_key, community_id, event_id, pseudonym_key),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Channel pinned messages.
CREATE TABLE IF NOT EXISTS channel_pins (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    pinned_by TEXT NOT NULL,
    pinned_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, community_id, channel_id, message_id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Game servers shared within a community.
CREATE TABLE IF NOT EXISTS game_servers (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    id TEXT NOT NULL,
    game_id TEXT NOT NULL,
    label TEXT NOT NULL,
    address TEXT NOT NULL,
    added_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, community_id, id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Locally cached invite codes (raw code never stored in DHT, only code_hash).
-- Populated when the current user creates an invite.
CREATE TABLE IF NOT EXISTS community_invites (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    code TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    -- VLD0 pointer to the single-owner DFLT record holding this invite's
    -- encrypted InviteSecrets blob. Surfaced into the deep link so the
    -- joiner fetches+decrypts secrets directly (chiral §12), without
    -- depending on the InviteCreated governance entry being visible.
    secrets_record_key TEXT NOT NULL DEFAULT '',
    max_uses INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    -- Architecture §16 — local uses counter incremented every time a
    -- `MemberJoinRequest` with this code_hash validates against a
    -- governance entry we hold. Non-authoritative: each peer counts
    -- only the requests they personally observed via gossip; the
    -- displayed value is a best-effort UX hint, not an enforcement
    -- threshold (the `max_uses` policy is enforced by the inviter at
    -- write time, not by reader consensus).
    uses INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, community_id, code_hash),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Per-message per-peer delivery tracking (Xfire imindex + SimpleX delivery states)
CREATE TABLE IF NOT EXISTS message_delivery (
    owner_key TEXT NOT NULL,
    message_id TEXT NOT NULL,
    community_id TEXT NOT NULL,
    recipient_pseudonym TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'sending' CHECK(status IN ('sending', 'delivered', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_attempt_at INTEGER,
    PRIMARY KEY (owner_key, message_id, recipient_pseudonym),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Strand Relay Network (architecture §13). For each friend who has volunteered
-- to relay traffic to us, we keep their dedicated relay route blob locally so
-- we can publish a padded relay pool subkey on our profile DHT record. Bob
-- adds rows here when he receives a `RelayOffer` from Carol.
CREATE TABLE IF NOT EXISTS strand_relay_offers (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    relay_pseudonym TEXT NOT NULL,
    relay_route_blob BLOB NOT NULL,
    received_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, relay_pseudonym)
);

-- Strand Relay outbound: when *we* volunteer to relay for a friend, we keep
-- the dedicated route id locally so we can revoke it later and so we know
-- which friend a given inbound RelayEnvelope is meant for (matched by route id).
CREATE TABLE IF NOT EXISTS strand_relay_volunteered (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    friend_public_key TEXT NOT NULL,
    relay_route_id TEXT NOT NULL,
    relay_route_blob BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, friend_public_key)
);

-- Direct messages and group DMs (architecture §27). Each row is one DM
-- conversation backed by a 2-or-N-member SMPL record (o_cnt:0). For
-- 2-party DMs `mek_generation` indexes the ratchet; for group DMs it
-- mirrors the broadcast generation. `participants_json` is a JSON
-- array of {pseudonym, subkey, public_key} for group DMs; for 2-party
-- DMs it has exactly two entries.
CREATE TABLE IF NOT EXISTS dms (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    record_key TEXT NOT NULL,
    is_group INTEGER NOT NULL DEFAULT 0,
    initiator_public_key TEXT NOT NULL,
    initiator_pseudonym TEXT NOT NULL,
    my_subkey INTEGER NOT NULL,
    participants_json TEXT NOT NULL CHECK(json_valid(participants_json)),
    -- 32-byte slot seed (hex) used to derive the SMPL slot keypairs for
    -- both peers. Persisted so writes survive a restart without re-
    -- exchanging the seed.
    slot_seed_hex TEXT NOT NULL DEFAULT '',
    -- Group DMs only (architecture §27.2): the wrapped MEK envelope
    -- (X25519+AES-256-GCM) addressed to *us*. Stashed at receive time
    -- so the user can defer accept; unwrapped on accept and dropped
    -- after the plaintext MEK lands in `dm_mek_cache`. For 2-party DMs
    -- this column is empty (MEK is re-derived from identity on demand).
    wrapped_mek_blob BLOB,
    mek_generation INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_message_at INTEGER,
    PRIMARY KEY (owner_key, record_key)
);

-- Local message log for DMs. Mirrors `messages` but per-DM (not per-friend)
-- since a single friend can have multiple DM conversations across pseudonym
-- contexts (architecture §27.1 unlinkability).
CREATE TABLE IF NOT EXISTS dm_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    record_key TEXT NOT NULL,
    sender_pseudonym TEXT NOT NULL,
    body TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    sequence INTEGER NOT NULL DEFAULT 0,
    mek_generation INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_dm_messages_record_ts
    ON dm_messages(owner_key, record_key, timestamp);

-- Mutual Aid topology metrics (architecture §14.5). Per-community
-- per-peer success/failure counters used to weight gossip fan-out
-- selection so high-reliability "ziplines" emerge organically. Pure
-- numeric counters; no PII beyond pseudonyms which are already public
-- within a community.
CREATE TABLE IF NOT EXISTS peer_reliability (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    community_id TEXT NOT NULL,
    peer_pseudonym TEXT NOT NULL,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, community_id, peer_pseudonym),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Mobile push relay registrations (architecture §17.3 Tier 3). A self-
-- hosted or shared headless `veilid-server` watches the listed DHT
-- record keys on this device's behalf and pings FCM/APNs with a
-- content-free wake. Stored locally so the client can re-register or
-- revoke from any session.
CREATE TABLE IF NOT EXISTS push_relay_registrations (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    relay_pseudonym TEXT NOT NULL,
    device_push_token TEXT NOT NULL,
    platform TEXT NOT NULL CHECK(platform IN ('fcm', 'apns', 'self')),
    record_keys_json TEXT NOT NULL CHECK(json_valid(record_keys_json)),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, relay_pseudonym)
);

-- Community member departure log (architecture §24.1). The live
-- membership table loses rows when a member leaves; analytics need a
-- separate append-only log so "leaves in last 7 days" stays computable.
CREATE TABLE IF NOT EXISTS community_member_leaves (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    community_id TEXT NOT NULL,
    pseudonym_key TEXT NOT NULL,
    left_at INTEGER NOT NULL,
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_member_leaves_recent
    ON community_member_leaves(owner_key, community_id, left_at);

-- Voice session join/leave log (architecture §24.1). Used to compute
-- "peak concurrent voice" per channel — events stream in as voice
-- sessions start and stop, the analytics query replays them as a sweep
-- to find the maximum concurrent count.
CREATE TABLE IF NOT EXISTS voice_session_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    member_pseudonym TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK(event_type IN ('join', 'leave')),
    occurred_at INTEGER NOT NULL,
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_voice_session_events_channel
    ON voice_session_events(owner_key, community_id, channel_id, occurred_at);

-- Local cache of devices paired to this identity (architecture §28.4).
-- The authoritative list lives in subkey 3 of the personal DFLT record;
-- this is just a fast read/write target for the settings UI.
CREATE TABLE IF NOT EXISTS paired_devices (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    device_public_key TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    paired_at INTEGER NOT NULL,
    unpaired_at INTEGER,
    PRIMARY KEY (owner_key, device_id)
);

-- Read-state mirror — this device's view of "last read Lamport per
-- channel". Reconciled into subkey 1 of the personal DFLT record.
CREATE TABLE IF NOT EXISTS channel_read_state (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    last_read_lamport INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, community_id, channel_id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Pending pairing sessions — short-lived rows the existing device
-- writes when generating a code, the new device drops as it accepts.
CREATE TABLE IF NOT EXISTS pending_pairings (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    pairing_code TEXT NOT NULL,
    pairing_salt BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, pairing_code)
);

-- Warm local cache of the raw per-author governance entry set merged
-- from the SMPL governance record (architecture v2.0 flat governance).
-- The DHT is the source of truth; this is the lossless merge *input*
-- (Vec<(PseudonymKey, Vec<GovernanceEntry>)> as JSON) so the next login
-- re-merges an identical GovernanceState immediately instead of waiting
-- on the slow best-effort DHT rebuild pass. A denormalized view of the
-- merged state lives in the communities/channels/roles tables; this is
-- the only place the full per-author entry set survives a restart.
CREATE TABLE IF NOT EXISTS governance_entries_cache (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    community_id TEXT NOT NULL,
    entries_json TEXT NOT NULL CHECK(json_valid(entries_json)),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, community_id),
    FOREIGN KEY (owner_key, community_id) REFERENCES communities(owner_key, id) ON DELETE CASCADE
);

-- Durable DHT writes a logout left unsent (plan C7.6h): the record pool's
-- held writes (own records that missed consensus, or whose wait the logout
-- released), re-pushed by the next login of the same identity. Signal's
-- DB-backed job queue, matrix-rust-sdk's persisted send queue. No writer
-- key is stored: every durable write targets a record the session holds
-- writable through its own lease.
CREATE TABLE IF NOT EXISTS dht_outbox (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    record_key TEXT NOT NULL,
    subkey INTEGER NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (owner_key, record_key, subkey)
);

-- The outbox (plan E2.5, D22; `rekindle_outbox`, `rekindle_db::repo::
-- outbox`): one row per send intent not yet settled. `stream` names an
-- ordered stream (`<kind>:<scope>`), `stream_seq` the row's place in it;
-- the stream's head (its lowest row not yet sent, or wedged `failed`)
-- blocks the rows after it. `client_id` is the id the frame carries, kept
-- across every attempt and a Retry; `messages.send_state` of the row
-- whose `message_id` it is mirrors `state`. Times are ms since the epoch;
-- `deadline` is `DELIVERY_HORIZON` (D24) after the enqueue unless the
-- stream sets a shorter one. A settled row is deleted.
CREATE TABLE IF NOT EXISTS outbox (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    stream TEXT NOT NULL CHECK(length(stream) > 0),
    stream_seq INTEGER NOT NULL CHECK(stream_seq >= 1),
    client_id TEXT NOT NULL CHECK(length(client_id) > 0),
    kind TEXT NOT NULL
        CHECK(kind IN ('dm', 'friend_handshake', 'dht_write', 'channel_post', 'media_key')),
    payload BLOB NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued', 'sent', 'delivered', 'read', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    next_at INTEGER NOT NULL,
    deadline INTEGER NOT NULL,
    wedge_reason TEXT,
    -- The local echo this row projects onto (`messages.send_state`);
    -- NULL for a send with no echo (a DHT write).
    echo_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    PRIMARY KEY (owner_key, stream, stream_seq),
    UNIQUE (owner_key, client_id)
);

CREATE INDEX IF NOT EXISTS idx_outbox_next_at ON outbox (owner_key, next_at);

-- Each stream's last assigned `stream_seq`: a settled row is deleted, and
-- a seq is never handed out twice (the peer acknowledges by it).
CREATE TABLE IF NOT EXISTS outbox_streams (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    stream TEXT NOT NULL,
    last_seq INTEGER NOT NULL CHECK(last_seq >= 1),
    PRIMARY KEY (owner_key, stream)
);

-- Client ids already applied from each peer stream (plan E2.5, D22): a
-- resend of a frame we hold is acknowledged, not shown twice.
CREATE TABLE IF NOT EXISTS received (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    stream TEXT NOT NULL,
    client_id TEXT NOT NULL,
    PRIMARY KEY (owner_key, stream, client_id)
);

-- The 1:1 ratchet (plan E2.3; `rekindle_db::ratchet`). One
-- `rekindle_crypto::SessionRecord` per peer device, stored in the same
-- transaction as whatever the message changed, before any send.
-- `peer_identity` is the peer's Ed25519 identity (hex), which is also its
-- address; `peer_device` its device id (hex), empty until its device list
-- (E2.4) names one.
CREATE TABLE IF NOT EXISTS ratchet_sessions (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer_identity TEXT NOT NULL,
    peer_device TEXT NOT NULL,
    record BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, peer_identity, peer_device)
);

-- The session resets we asked friends for (plan audit D-4): consumed in the
-- transaction that accepts the friend's SessionResetAccept init, deleted on
-- their decline, expired at the backend-hold lifetime.
CREATE TABLE IF NOT EXISTS session_reset_requests (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer TEXT NOT NULL,
    requested_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, peer)
);

-- Our prekeys (plan E2.3, E2.4; `rekindle_db::prekeys`): the signed
-- prekey (`spk`) and the ML-KEM last-resort key (`pq_lr`), published in
-- the prekey record's head (subkey 0), and the one-time keys (`opk`,
-- `pq_opk`), a pair per slot (subkeys 1..100). `published_subkey` is the
-- record subkey holding the key. Ids are random non-zero u31 (PQXDH
-- §4.13). A one-time key is consumed only after the first message under
-- it decrypts: `consumed_at` is set and its secret erased. A replaced key
-- gets `retired_at` and stays usable; the archive sweep deletes a retired
-- or consumed row 30 days on.
CREATE TABLE IF NOT EXISTS signal_prekeys (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    id INTEGER NOT NULL CHECK(id BETWEEN 1 AND 2147483647),
    kind TEXT NOT NULL CHECK(kind IN ('opk', 'spk', 'pq_opk', 'pq_lr')),
    secret BLOB NOT NULL,
    public BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    published_subkey INTEGER NOT NULL CHECK(published_subkey BETWEEN 0 AND 100),
    consumed_at INTEGER,
    retired_at INTEGER,
    PRIMARY KEY (owner_key, kind, id)
);

-- The one-time slots we initiated from, per peer device (plan E2.4 B2):
-- the pick skips them. A row is swept once the peer's record no longer
-- lists the id, or by the archive sweep.
CREATE TABLE IF NOT EXISTS prekey_slots_used (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer TEXT NOT NULL,
    device_id TEXT NOT NULL,
    opk_id INTEGER NOT NULL CHECK(opk_id BETWEEN 1 AND 2147483647),
    used_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, peer, device_id, opk_id)
);

-- The `PreKeyRejected` notices we sent (plan E2.4 B5): one per initiator
-- base key (`base_key_hash` = BLAKE3 derive-key under
-- `PREKEY_REJECTED_BASE_KEY`, `prekey_record::base_key_hash`), at most 10 per sender per
-- 3 h. Kept for the 30-day archive horizon.
CREATE TABLE IF NOT EXISTS prekey_rejections (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    sender TEXT NOT NULL,
    base_key_hash BLOB NOT NULL CHECK(length(base_key_hash) = 32),
    sent_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, base_key_hash)
);

-- The PreKeyInits that opened a session on the last-resort key (plan
-- E2.4 B8, B9): an LR-only init seen again is a replay and is dropped.
-- Kept while the last-resort key is held.
CREATE TABLE IF NOT EXISTS prekey_lr_inits (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    lr_id INTEGER NOT NULL CHECK(lr_id BETWEEN 1 AND 2147483647),
    spk_id INTEGER NOT NULL CHECK(spk_id BETWEEN 1 AND 2147483647),
    base_key BLOB NOT NULL CHECK(length(base_key) = 32),
    seen_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, lr_id, spk_id, base_key)
);

-- Peers' identity keys and their verification (plan E2.3;
-- `rekindle_db::repo::signal_identities`). `changed_from_key` and
-- `changed_at` hold an unacknowledged change of a verified key (the
-- `VerifiedKeyChanged` escalation); committing the escalated action
-- clears them.
CREATE TABLE IF NOT EXISTS signal_identities (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer_identity TEXT NOT NULL,
    identity_key BLOB NOT NULL
        CHECK(typeof(identity_key) = 'blob' AND length(identity_key) = 32),
    verified INTEGER NOT NULL DEFAULT 0 CHECK(verified IN (0, 1)),
    verified_at INTEGER CHECK(verified_at IS NULL OR verified_at >= 0),
    changed_from_key BLOB CHECK(changed_from_key IS NULL
        OR (typeof(changed_from_key) = 'blob' AND length(changed_from_key) = 32)),
    changed_at INTEGER CHECK(changed_at IS NULL OR changed_at >= 0),
    PRIMARY KEY (owner_key, peer_identity)
);

-- Decrypted, not yet applied 1:1 messages (plan E2.3, D24): written in
-- the ratchet's transaction, deleted by the apply, replayed at startup.
-- Keyed by the ratchet header, unique per message per session. Rows older
-- than `UNPROCESSED_MAX_AGE` (from `DELIVERY_HORIZON`) or tried
-- `MAX_UNPROCESSED_ATTEMPTS` times are dropped at replay. A row that does
-- not read back is quarantined (`quarantined_at`): skipped by every later
-- replay, so it never blocks the others, and swept by the same age bound.
CREATE TABLE IF NOT EXISTS unprocessed (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer_identity TEXT NOT NULL,
    header_dh BLOB NOT NULL CHECK(typeof(header_dh) = 'blob' AND length(header_dh) = 32),
    header_n INTEGER NOT NULL CHECK(header_n BETWEEN 0 AND 4294967295),
    type_id INTEGER NOT NULL CHECK(type_id BETWEEN 0 AND 65535),
    plaintext BLOB NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    received_at INTEGER NOT NULL CHECK(received_at >= 0),
    quarantined_at INTEGER CHECK(quarantined_at IS NULL OR quarantined_at >= 0),
    PRIMARY KEY (owner_key, peer_identity, header_dh, header_n)
);

-- The send log (plan E2.5, D24 send half): every user-content ratchet
-- message we sent, written in the ratchet's transaction, so a peer that
-- could not open it can ask for it again (`RetryRequest`). `msg_id` is the
-- message's header id (`rekindle_crypto::replay::ratchet_message_id`),
-- `ratchet_key` the header's sending ratchet key. `plaintext_sealed` is
-- the frame plaintext, sealed at rest by this database's SQLCipher key
-- (D1/D3). A request whose observed key matches resends it under the
-- current session and archives the row (`archived_at`); rows older than
-- `DELIVERY_HORIZON` are pruned.
CREATE TABLE IF NOT EXISTS message_send_log (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer TEXT NOT NULL,
    msg_id BLOB NOT NULL CHECK(typeof(msg_id) = 'blob' AND length(msg_id) = 32),
    type_id INTEGER NOT NULL CHECK(type_id BETWEEN 0 AND 65535),
    ratchet_key BLOB NOT NULL CHECK(typeof(ratchet_key) = 'blob' AND length(ratchet_key) = 32),
    plaintext_sealed BLOB NOT NULL,
    sent_at INTEGER NOT NULL CHECK(sent_at >= 0),
    archived_at INTEGER CHECK(archived_at IS NULL OR archived_at >= 0),
    PRIMARY KEY (owner_key, peer, msg_id)
);

-- Our retry requests (plan E2.5, D24): a ratchet message from `peer` we
-- could not open, held (`pending`) until the source's CaughtUp, then sent
-- at most `MAX_PER_MESSAGE` times per message (`sent_count`) and
-- `MAX_PER_PEER` times per peer per `PEER_WINDOW` (`retry_request_sends`).
CREATE TABLE IF NOT EXISTS retry_requests (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer TEXT NOT NULL,
    msg_id BLOB NOT NULL CHECK(typeof(msg_id) = 'blob' AND length(msg_id) = 32),
    observed_ratchet_key BLOB NOT NULL CHECK(typeof(observed_ratchet_key) = 'blob' AND length(observed_ratchet_key) = 32),
    pending INTEGER NOT NULL CHECK(pending IN (0, 1)),
    sent_count INTEGER NOT NULL DEFAULT 0 CHECK(sent_count >= 0),
    failed_at INTEGER NOT NULL CHECK(failed_at >= 0),
    PRIMARY KEY (owner_key, peer, msg_id)
);

-- Each retry request sent, for the per-peer window (plan E2.5).
CREATE TABLE IF NOT EXISTS retry_request_sends (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    peer TEXT NOT NULL,
    sent_at INTEGER NOT NULL CHECK(sent_at >= 0)
);
CREATE INDEX IF NOT EXISTS idx_retry_request_sends_peer
    ON retry_request_sends(owner_key, peer, sent_at);

-- FTS5 indexes for local search (architecture §23). External-content tables
-- per https://sqlite.org/fts5.html#external_content_tables — index lives in
-- a parallel virtual table, content stays in the canonical row table, and
-- AFTER INSERT/UPDATE/DELETE triggers keep them in lock-step.
--
-- Tokenizer: `unicode61 remove_diacritics 2` — Unicode-aware splitting on
-- punctuation/whitespace with strict diacritic folding (the "form 2"
-- variant the FTS5 reference recommends for case- and accent-insensitive
-- matching). No `porter` stemming because it's English-only and Rekindle
-- targets multilingual chat. bm25() ranking is built-in.
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    body,
    content='messages',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, body) VALUES (new.id, new.body);
END;
CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, body) VALUES('delete', old.id, old.body);
END;
CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, body) VALUES('delete', old.id, old.body);
    INSERT INTO messages_fts(rowid, body) VALUES (new.id, new.body);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS thread_messages_fts USING fts5(
    body,
    content='thread_messages',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS thread_messages_fts_ai AFTER INSERT ON thread_messages BEGIN
    INSERT INTO thread_messages_fts(rowid, body) VALUES (new.id, new.body);
END;
CREATE TRIGGER IF NOT EXISTS thread_messages_fts_ad AFTER DELETE ON thread_messages BEGIN
    INSERT INTO thread_messages_fts(thread_messages_fts, rowid, body) VALUES('delete', old.id, old.body);
END;
CREATE TRIGGER IF NOT EXISTS thread_messages_fts_au AFTER UPDATE ON thread_messages BEGIN
    INSERT INTO thread_messages_fts(thread_messages_fts, rowid, body) VALUES('delete', old.id, old.body);
    INSERT INTO thread_messages_fts(rowid, body) VALUES (new.id, new.body);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS dm_messages_fts USING fts5(
    body,
    content='dm_messages',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS dm_messages_fts_ai AFTER INSERT ON dm_messages BEGIN
    INSERT INTO dm_messages_fts(rowid, body) VALUES (new.id, new.body);
END;
CREATE TRIGGER IF NOT EXISTS dm_messages_fts_ad AFTER DELETE ON dm_messages BEGIN
    INSERT INTO dm_messages_fts(dm_messages_fts, rowid, body) VALUES('delete', old.id, old.body);
END;
CREATE TRIGGER IF NOT EXISTS dm_messages_fts_au AFTER UPDATE ON dm_messages BEGIN
    INSERT INTO dm_messages_fts(dm_messages_fts, rowid, body) VALUES('delete', old.id, old.body);
    INSERT INTO dm_messages_fts(rowid, body) VALUES (new.id, new.body);
END;
