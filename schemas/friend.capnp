@0xeb1413f2ea507df6;

struct FriendRequest {
    senderKey @0 :Data;
    displayName @1 :Text;
    message @2 :Text;
    preKeyBundle @3 :Data;       # For immediate session setup
}

struct FriendList {
    friends @0 :List(FriendEntry);
}

struct FriendEntry {
    publicKey @0 :Data;
    nickname @1 :Text;
    groupName @2 :Text;
    addedAt @3 :UInt64;
    # The Rust struct carried these two and the schema did not, so the
    # encoder dropped them on every write: `profileDhtKey` silently (the
    # decoder even set it to None with a comment saying why), and
    # `dmLogKey` because the daemon track had added it to its own copy
    # of FriendEntry while the desktop encoded this one.
    profileDhtKey @4 :Text;
    dmLogKey @5 :Text;
}
