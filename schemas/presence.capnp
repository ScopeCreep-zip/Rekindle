@0x99e0a9dc9c41c978;

struct PresenceUpdate {
    status @0 :UInt8;            # 0=online, 1=away, 2=busy, 3=offline
    gameStatus @1 :GameStatus;
    timestamp @2 :UInt64;
}

struct GameStatus {
    gameId @0 :UInt32;
    gameName @1 :Text;
    serverInfo @2 :Text;         # "map_name @ server_ip:port"
    elapsedSeconds @3 :UInt32;
}
