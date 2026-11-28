@0xeb1413f2ea507df6;

struct FriendList {
    friends @0 :List(FriendEntry);
}

struct FriendEntry {
    publicKey @0 :Data;
    nickname @1 :Text;
    groupName @2 :Text;
    addedAt @3 :UInt64;
    profileDhtKey @4 :Text;
    # Was `dmLogKey`, the daemon's per-friend DM log, deleted with the log
    # (plan E2.1); the ordinal stays reserved and is never written.
    obsoleteDmLogKey @5 :Text;
}
