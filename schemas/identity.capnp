@0xf1e08b0e35343cb1;

using Presence = import "presence.capnp";

struct UserProfile {
    displayName @0 :Text;
    statusMessage @1 :Text;
    status @2 :Status;
    avatarHash @3 :Data;
    gameStatus @4 :Presence.GameStatus;

    enum Status {
        online @0;
        away @1;
        busy @2;
        offline @3;
    }
}

struct PreKeyBundle {
    identityKey @0 :Data;        # Ed25519 public key
    signedPreKey @1 :Data;       # X25519 signed prekey
    signedPreKeySig @2 :Data;    # Signature over signed prekey
    oneTimePreKey @3 :Data;      # Optional one-time prekey
    registrationId @4 :UInt32;
}
