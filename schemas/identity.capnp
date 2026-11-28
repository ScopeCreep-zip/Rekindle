@0xf1e08b0e35343cb1;

# The prekey record (plan E2.4): a DFLT(101) record per device, owned by
# a keypair of its own and named from profile subkey 5. Every id is a
# random non-zero u31 (PQXDH §4.13), and every signature is by the
# identity key over `id ‖ key` under a domain tag
# (`rekindle_crypto::signal::pqxdh::verify`).

# Subkey 0: the signed prekey and the ML-KEM last-resort key.
struct PrekeyRecordHead {
    spkId @0 :UInt32;
    spk @1 :Data;                # X25519 public key (32 B)
    spkSig @2 :Data;             # Ed25519 over 0x01 ‖ u32be(spkId) ‖ EncodeEC(spk)
    pqLrId @3 :UInt32;
    pqLr @4 :Data;               # ML-KEM-768 public key (1184 B)
    pqLrSig @5 :Data;            # Ed25519 over 0x02 ‖ "LR" ‖ u32be(pqLrId) ‖ EncodeKEM(pqLr)
    issuedAt @6 :Int64;          # Unix ms the head was minted (the rows' `created_at`)
}

# Subkeys 1..100: one one-time curve prekey and one one-time ML-KEM key.
struct OpkSlot {
    opkId @0 :UInt32;
    opk @1 :Data;                # X25519 public key (32 B)
    opkSig @2 :Data;             # Ed25519 over 0x03 ‖ u32be(opkId) ‖ EncodeEC(opk)
    pqOpkId @3 :UInt32;
    pqOpk @4 :Data;              # ML-KEM-768 public key (1184 B)
    pqOpkSig @5 :Data;           # Ed25519 over 0x02 ‖ "OT" ‖ u32be(pqOpkId) ‖ EncodeKEM(pqOpk)
}

# Profile subkey 5: each device's prekey record (one entry until E5.3).
struct PrekeyRecordList {
    devices @0 :List(PrekeyDevice);
}

struct PrekeyDevice {
    deviceId @0 :Data;           # the identity's 16-byte random `identity.device_id`
    recordKey @1 :Text;          # the DFLT(101) record's key
}
