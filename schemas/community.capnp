@0xe49f21cc1dd1b25b;

struct Community {
    name @0 :Text;
    description @1 :Text;
    iconHash @2 :Data;
    createdAt @3 :UInt64;
    channels @4 :List(Channel);
    roles @5 :List(Role);
}

struct Channel {
    id @0 :Text;
    name @1 :Text;
    type @2 :ChannelType;
    sortOrder @3 :UInt16;
    latestMessageKey @4 :Data;   # DHT record key for message chain

    enum ChannelType {
        text @0;
        voice @1;
    }
}

struct Role {
    id @0 :UInt32;
    name @1 :Text;
    color @2 :UInt32;            # RGB packed
    permissions @3 :UInt64;      # Bitmask
    sortOrder @4 :UInt16;
}
