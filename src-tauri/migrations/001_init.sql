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
    mailbox_dht_key TEXT
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
    PRIMARY KEY (owner_key, public_key)
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    conversation_type TEXT NOT NULL CHECK(conversation_type IN ('dm', 'channel')),
    sender_key TEXT NOT NULL,
    body TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    is_read INTEGER NOT NULL DEFAULT 0,
    reply_to_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    attachment_json TEXT,
    mek_generation INTEGER
);

CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(owner_key, conversation_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_unread ON messages(owner_key, conversation_id, is_read) WHERE is_read = 0;
CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_dedup
  ON messages(owner_key, conversation_id, conversation_type, sender_key, timestamp);

CREATE TABLE IF NOT EXISTS communities (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    icon_hash TEXT,
    my_role TEXT NOT NULL DEFAULT 'member',
    my_role_ids TEXT NOT NULL DEFAULT '[0,1]',
    joined_at INTEGER NOT NULL,
    last_synced INTEGER,
    dht_record_key TEXT,
    dht_owner_keypair TEXT,
    my_pseudonym_key TEXT,
    mek_generation INTEGER NOT NULL DEFAULT 0,
    server_route_blob BLOB,
    is_hosted INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (owner_key, id)
);

CREATE TABLE IF NOT EXISTS channels (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    id TEXT NOT NULL,
    community_id TEXT NOT NULL,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL CHECK(channel_type IN ('text', 'voice')),
    sort_order INTEGER NOT NULL DEFAULT 0,
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
    PRIMARY KEY (owner_key, community_id, role_id)
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

CREATE TABLE IF NOT EXISTS blocked_users (
    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
    public_key TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    blocked_at INTEGER NOT NULL,
    PRIMARY KEY (owner_key, public_key)
);

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
