# Architecture §27 — community-favourited game-server metadata,
# broadcast over the `GameServerAdded` control envelope.
@0xe6997951e61b3351;

struct GameServerInfo @0xa2d4eb16e9bf1c2e {
    # 16-byte UUID hex (`gs_<32 hex>`).
    id              @0 :Text;
    # Game identifier (rich-presence id).
    gameId          @1 :Text;
    label           @2 :Text;
    # `host:port` string the launcher uses.
    address         @3 :Text;
    # Hex-encoded pseudonym of the member who added the server.
    addedBy         @4 :Text;
    createdAt       @5 :UInt64;
}
