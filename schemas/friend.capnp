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
}
