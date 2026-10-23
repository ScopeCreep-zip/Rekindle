# Architecture §15 — channel-message snapshots carried inside
# `SyncResponse.messages`.
@0xd2d8721ffbb37e44;

# Architecture §15 — sync-response message entry. It is pulled from SQLite, where the
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
