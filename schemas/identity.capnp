@0xf1e08b0e35343cb1;

struct PreKeyBundle {
    # Classical X3DH layer.
    identityKey @0 :Data;        # Ed25519 public key
    signedPreKey @1 :Data;       # X25519 signed prekey
    signedPreKeySig @2 :Data;    # Ed25519 signature over 0x01 || signedPreKey
    oneTimePreKey @3 :Data;      # Optional X25519 one-time prekey (empty if absent)
    registrationId @4 :UInt32;

    # PQXDH layer (Phase 3b of decomposed-harvest plan). Sticky field
    # numbers — never reused.
    pqpkLr @5 :Data;             # ML-KEM-768 last-resort public key (1184 B)
    pqpkLrSig @6 :Data;          # Ed25519 signature over 0x02 || "LR" || pqpkLr
    pqpkOt @7 :Data;             # Optional one-time ML-KEM-768 public (1184 B, empty if absent)
    pqpkOtSig @8 :Data;          # Ed25519 signature over 0x02 || "OT" || pqpkOt
    pqpkOtId @9 :UInt32;         # One-time PQ prekey identifier (0 if absent)
    oneTimePreKeyId @10 :UInt32; # One-time X25519 prekey identifier (0 if absent)
}
