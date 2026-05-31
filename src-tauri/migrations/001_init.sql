-- Rekindle initial schema migration
-- Creates all core tables for identity, friends, messaging, communities, and Signal protocol state.

CREATE TABLE IF NOT EXISTS identity (
    id INTEGER PRIMARY KEY,
    public_key TEXT NOT NULL UNIQUE,
    display_name TEXT,
    created_at INTEGER NOT NULL,
    dht_record_key TEXT,
    dht_owner_keypair TEXT,
    friend_list_dht_key TEXT,
    friend_list_owner_keypair TEXT,
    avatar_webp BLOB,
    account_dht_key TEXT,
    account_owner_keypair TEXT,
    mailbox_dht_key TEXT,
    -- Architecture §28.4: personal DFLT record key for cross-device sync.
    -- Created lazily on first launch when the user opts into multi-device.
    personal_sync_record_key TEXT,
    personal_sync_owner_keypair TEXT,
    -- Stable per-device id (random 16-byte UUID hex). Identifies *this*
    -- physical device in the DeviceList without exposing more PII.
    device_id TEXT
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
    local_conversation_key TEXT,
    local_conversation_keypair TEXT,
    remote_conversation_key TEXT,
    mailbox_dht_key TEXT,
    friendship_state TEXT NOT NULL DEFAULT 'accepted',
    -- Phase 2 Track A.2 — Friend's most-recently-seen DeviceId
    -- (hex Ed25519 device pubkey). NULL until they advertise via the
    -- DeviceList owner subkey on their per-pair inbox.
    current_device_id TEXT,
    PRIMARY KEY (owner_key, public_key)
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
    attachment_json TEXT,
    mek_generation INTEGER,
    lamport_ts INTEGER DEFAULT 0,
    message_id TEXT,
    forwarded_from_author TEXT,
    flags INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(owner_key, conversation_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_unread ON messages(owner_key, conversation_id, is_read) WHERE is_read = 0;
CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_dedup
  ON messages(owner_key, conversation_id, conversation_type, sender_key, timestamp);
CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_message_id
  ON messages(owner_key, conversation_id, message_id) WHERE message_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS communities (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    icon_hash TEXT,
    banner_hash TEXT,
    my_role_ids TEXT NOT NULL DEFAULT '[0]',
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
    coordinator_pseudonym TEXT,
    coordinator_route_blob BLOB,
    coordinator_epoch INTEGER NOT NULL DEFAULT 0,
    governance_key TEXT,
    lamport_clock INTEGER NOT NULL DEFAULT 0,
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
    role_ids TEXT NOT NULL DEFAULT '[0,1]',
    timeout_until INTEGER,
    joined_at INTEGER NOT NULL,
    subkey_index INTEGER,
    -- Plate Gate (architecture §15): which segment hosts this member's slot.
    -- 0 = primary segment (the genesis registry); 1..=MAX_SEGMENTS for each
    -- expansion. The local subkey within that segment's registry record is
    -- always `subkey_index - segment_index * 255`.
    segment_index INTEGER NOT NULL DEFAULT 0,
    onboarding_complete INTEGER NOT NULL DEFAULT 0,
    -- Per-community profile fields (architecture §8.2 / §24.2). Mirrored
    -- from peer MemberPresence at registry-poll time so they survive
    -- restarts; without these the member list shows pseudonym keys
    -- after a relaunch.
    bio TEXT,
    pronouns TEXT,
    theme_color INTEGER,
    badges TEXT NOT NULL DEFAULT '[]',
    -- BLAKE3 content references for avatar + banner. The bytes
    -- themselves live in the local Lost Cargo cache or as an inline
    -- expression asset (architecture §32 Week 15).
    avatar_ref TEXT,
    banner_ref TEXT,
    PRIMARY KEY (owner_key, community_id, pseudonym_key)
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
    PRIMARY KEY (owner_key, community_id, role_id)
);

CREATE TABLE IF NOT EXISTS community_categories (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, community_id, id)
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
    PRIMARY KEY (owner_key, community_id, channel_id, target_type, target_id)
);

-- Performance indexes for JOIN keys
CREATE INDEX IF NOT EXISTS idx_friends_group_id ON friends(owner_key, group_id);
CREATE INDEX IF NOT EXISTS idx_channels_community_id ON channels(owner_key, community_id);
CREATE INDEX IF NOT EXISTS idx_community_members_community ON community_members(owner_key, community_id, pseudonym_key);

CREATE TABLE IF NOT EXISTS trusted_identities (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    public_key TEXT NOT NULL,
    identity_key BLOB NOT NULL,
    verified INTEGER NOT NULL DEFAULT 0,
    first_seen INTEGER NOT NULL,
    PRIMARY KEY (owner_key, public_key)
);

CREATE TABLE IF NOT EXISTS signal_sessions (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    recipient_key TEXT NOT NULL,
    session_data BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, recipient_key)
);

CREATE TABLE IF NOT EXISTS prekeys (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    id INTEGER NOT NULL,
    key_data BLOB NOT NULL,
    is_signed INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, id)
);

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
    payload_json TEXT NOT NULL,
    PRIMARY KEY (owner_key, cursor)
);

-- W16.1 / W16.9 — unified envelope retry queue (the rekindle-transport
-- `EnvelopeStore` trait persisted via SQLite for the Tauri shell).
-- Replaces the legacy `pending_messages` use for envelope retries
-- (DM body / friend-add / call signaling / DM invite). Per-kind retry
-- caps are stored on the row so the queue's tick loop can enforce
-- them without the per-kind config being centralized in SQL.
CREATE TABLE IF NOT EXISTS pending_envelopes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    recipient_key TEXT NOT NULL,
    envelope_kind TEXT NOT NULL,
    seq INTEGER NOT NULL,
    correlation_id TEXT,
    payload BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL,
    next_retry_at_ms INTEGER NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL,
    last_error TEXT
);
CREATE INDEX IF NOT EXISTS idx_pending_envelopes_eligible
    ON pending_envelopes (owner_key, next_retry_at_ms);
CREATE INDEX IF NOT EXISTS idx_pending_envelopes_correlation
    ON pending_envelopes (correlation_id);

-- W16.3 — receiver-side dedup state. Veilid `app_message` has no
-- built-in dedup; without this table a duplicate transport-level
-- retransmit would mount a second IncomingCallModal, etc.
CREATE TABLE IF NOT EXISTS seen_envelopes (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    sender_key TEXT NOT NULL,
    envelope_kind TEXT NOT NULL,
    correlation_id TEXT NOT NULL DEFAULT '',
    last_seq INTEGER NOT NULL,
    last_seen_at_ms INTEGER NOT NULL,
    PRIMARY KEY (owner_key, sender_key, envelope_kind, correlation_id)
);

-- W16.3 — sender-side seq tracking. Persisted so seq survives
-- restart; without this the post-crash seq could go backward and
-- the receiver would drop our legitimate envelopes as duplicates.
CREATE TABLE IF NOT EXISTS outbound_seqs (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    recipient_key TEXT NOT NULL,
    envelope_kind TEXT NOT NULL,
    correlation_id TEXT NOT NULL DEFAULT '',
    next_seq INTEGER NOT NULL,
    PRIMARY KEY (owner_key, recipient_key, envelope_kind, correlation_id)
);

-- W16.8 — Dialing/Incoming call state for crash recovery. Active
-- calls intentionally NOT persisted (matches Signal RingRTC + Discord
-- — voice transport state is process-bound and cannot meaningfully
-- resume). On startup the runtime's `recover()` loads these rows,
-- rehydrates the in-memory state machine, and either resumes the
-- ring (if expires_at_ms is still in the future) or persists a
-- missed-call notification (if it already expired).
CREATE TABLE IF NOT EXISTS active_call_states (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    call_id TEXT NOT NULL,
    peer_pubkey TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    my_x25519_secret BLOB,
    peer_x25519_pub BLOB,
    group_participants TEXT NOT NULL DEFAULT '',
    inserted_at_ms INTEGER NOT NULL,
    PRIMARY KEY (owner_key, call_id)
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
    prekey_bundle BLOB,
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
    PRIMARY KEY (owner_key, community_id, channel_id)
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
    -- remain bare in the chat. Default 1 (enabled) matches the typical
    -- chat-app expectation of inline previews.
    link_previews_enabled INTEGER NOT NULL DEFAULT 1
        CHECK(link_previews_enabled IN (0, 1)),
    PRIMARY KEY (owner_key)
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
    PRIMARY KEY (owner_key, community_id, channel_id)
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
    PRIMARY KEY (owner_key, community_id, id)
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
    UNIQUE (owner_key, community_id, thread_id, message_id)
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
    recurrence_json TEXT,
    -- Architecture §21 line 2629 — JSON-serialized EventLocation enum.
    -- Falls back to channel_id-based VoiceChannel for legacy rows.
    location_json TEXT,
    PRIMARY KEY (owner_key, community_id, id)
);

-- Event RSVPs (one per member per event).
CREATE TABLE IF NOT EXISTS event_rsvps (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    pseudonym_key TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'none',
    PRIMARY KEY (owner_key, community_id, event_id, pseudonym_key)
);

-- Local member-owned event RSVPs mirrored into presence writes.
CREATE TABLE IF NOT EXISTS community_event_rsvps (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'declined',
    PRIMARY KEY (owner_key, community_id, event_id)
);

-- Channel pinned messages.
CREATE TABLE IF NOT EXISTS channel_pins (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    pinned_by TEXT NOT NULL,
    pinned_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, community_id, channel_id, message_id)
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
    PRIMARY KEY (owner_key, community_id, id)
);

-- Locally cached invite codes (raw code never stored in DHT, only code_hash).
-- Populated when the current user creates an invite.
CREATE TABLE IF NOT EXISTS community_invites (
    owner_key TEXT NOT NULL,
    community_id TEXT NOT NULL,
    code TEXT NOT NULL,
    code_hash TEXT NOT NULL,
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
    PRIMARY KEY (owner_key, community_id, code_hash)
);

-- Per-message per-peer delivery tracking (Xfire imindex + SimpleX delivery states)
CREATE TABLE IF NOT EXISTS message_delivery (
    message_id TEXT NOT NULL,
    community_id TEXT NOT NULL,
    recipient_pseudonym TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'sending' CHECK(status IN ('sending', 'delivered', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_attempt_at INTEGER,
    PRIMARY KEY (message_id, recipient_pseudonym)
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
    participants_json TEXT NOT NULL,
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
    PRIMARY KEY (owner_key, community_id, peer_pseudonym)
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
    record_keys_json TEXT NOT NULL,
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
    left_at INTEGER NOT NULL
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
    occurred_at INTEGER NOT NULL
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
    PRIMARY KEY (owner_key, community_id, channel_id)
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
