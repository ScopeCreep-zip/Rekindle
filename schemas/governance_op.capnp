@0xccf28eb901c53531;

# A signed governance op (ADR 0011, plan E3.2): one entry in its author's
# op chain, citing the ops it causally follows. The author pseudonym signs
# `rekindle-gov-op-v1 ‖ community ‖ unsigned`, where `unsigned` is this
# message re-encoded canonically (packed, fields in schema order) with
# `sig` unset; the op hash is BLAKE3 derive_key(rekindle-gov-op-hash-v1)
# over `unsigned ‖ sig`. A retired field keeps its ordinal as `obsolete…`.

using Gov = import "community_governance.capnp";

# A locator for an op: enough to fetch it from its author's member record
# (`authorRecord`, at `authorSeq`) and to check what came back.
struct OpRef {
  opHash @0 :Data;             # 32 bytes
  author @1 :Data;             # the author pseudonym, 32 bytes
  authorRecord @2 :Text;       # DHT key of the author's member record
  authorSeq @3 :UInt64;
}

struct GovernanceOp {
  author @0 :Data;             # the author pseudonym, 32 bytes
  seq @1 :UInt64;              # position in the author's chain, from 0
  prev @2 :Data;               # the author's op at seq - 1; empty at seq 0
  deps @3 :List(OpRef);        # at most 64 (member_record::MAX_FRONTIER_REFS)
  issuedAt @4 :UInt64;         # author's seconds at issue; a tie-break only
  body @5 :Gov.GovernanceEntry;
  sig @6 :Data;                # Ed25519, 64 bytes; unset in the unsigned form
}

# The community's genesis certificate, the anchor's subkey 0 (plan E3.2,
# ADR 0011), written once. Both `anchorSig` (by the anchor's owner `K_A`,
# discarded after the write) and `creatorSig` (by the creator pseudonym) are
# Ed25519 over `rekindle-gov-genesis-v1 ‖ anchor ‖ unsigned`, where
# `unsigned` is this message re-encoded canonically with both sigs unset.
struct GenesisCert {
  name @0 :Text;
  policy @1 :Gov.AdmissionMode;
  initialRoles @2 :List(Data);         # 16-byte role ids
  creatorPseudonym @3 :Data;           # 32 bytes
  creatorMemberRecord @4 :Text;        # DHT key of the creator's M(p)
  creatorMemberRecordOwner @5 :Text;   # K_M(p)'s public key text
  createdAt @6 :UInt64;
  anchorOwner @7 :Data;                # K_A's public key, 32 bytes
  anchorSig @8 :Data;                  # 64 bytes; unset in the unsigned form
  creatorSig @9 :Data;                 # 64 bytes; unset in the unsigned form
}
