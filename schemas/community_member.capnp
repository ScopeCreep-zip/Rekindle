# Architecture §4.3 — flat member description used by gossip envelopes
# (`MemberJoined` broadcasts, `BootstrapResponse.member_list`).
#
# Wire-evolution rules: append-only ordinals (@N). Never reorder, never
# reuse, never remove. Old peers ignore ordinals they don't understand
# (Cap'n Proto schema evolution rules).
@0xd86bd6d581593507;

struct MemberInfo @0xa0d4eb16e9bf1c2e {
    # Hex-encoded community pseudonym Ed25519 public key (64-char hex).
    pseudonymKey   @0 :Text;
    displayName    @1 :Text;
    roleIds        @2 :List(UInt32);
    # `online` / `away` / `busy` / `offline`.
    status         @3 :Text;
    # Empty when route is omitted (offline / private routes withheld).
    routeBlob      @4 :Data;
    bio            @5 :Text;
    pronouns       @6 :Text;
    # 0 when no theme color set (caller treats 0 as "default").
    themeColor     @7 :UInt32;
    badges         @8 :List(Text);
    # Unix-seconds of the last presence heartbeat from this member.
    lastSeen       @9 :UInt64;
}
