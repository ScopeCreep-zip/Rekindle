# Architecture §13.4 / §15 — channel-message snapshots carried inside
# `BootstrapResponse.recentMessages` and `SyncResponse.messages`.
@0xd2d8721ffbb37e44;

# Architecture §13.4 line 2068 — `ciphertext` is freshly re-encrypted
# under the joiner's current MEK (the same key delivered alongside in
# `BootstrapResponse.channelMeks`).
struct BootstrapMessage @0xa4d4eb16e9bf1c2e {
    messageId        @0 :Text;
    # Hex-encoded sender pseudonym.
    senderPseudonym  @1 :Text;
    ciphertext       @2 :Data;
    mekGeneration    @3 :UInt64;
    # Stored as Int64 to preserve sign semantics from SQLite (timestamps
    # are usually positive but SQLite returns i64 native).
    timestamp        @4 :Int64;
}

# Architecture §13.4 — bootstrap snapshot grouped by channel so the
# joiner doesn't pay the per-message overhead of repeating channelId
# for every entry.
struct BootstrapChannelMessages @0xa5d4eb16e9bf1c2e {
    channelId        @0 :Text;
    messages         @1 :List(BootstrapMessage);
}

# Architecture §15 — sync-response message entry. Different shape from
# BootstrapMessage because it's pulled from SQLite, where the
# historical message rows carry the columns this struct mirrors.
struct SyncedMessage @0xa6d4eb16e9bf1c2e {
    # Hex-encoded sender community pseudonym.
    senderKey        @0 :Text;
    # Stored message body (architecture §15 line 2210 — already
    # MEK-encrypted at write time).
    body             @1 :Text;
    timestamp        @2 :Int64;
    # Optional in the JSON wire form. Cap'n Proto can't easily express
    # nullable scalars, so we use sentinel encoding: when `hasMekGeneration`
    # is false, treat `mekGeneration` as absent.
    hasMekGeneration @3 :Bool;
    mekGeneration    @4 :Int64;
    hasLamportTs     @5 :Bool;
    lamportTs        @6 :Int64;
}
