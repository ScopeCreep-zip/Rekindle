@0x86aaa87d6f051124;

# The 1:1 wire (plan E2.3, §6 `DmFrame` typeIds). A datagram is the `'D'`
# ingress tag, then one packed `DmFrame`. The frame's typeId names the
# kind and decides which body it carries:
#   - 0x0001 PreKeyInit: `preKeyInit`, whose encrypted first payload is
#     the kind `firstTypeId` names (FriendAccept 0x0101, SessionResetAccept
#     0x0008, or any ratchet kind);
#   - the pre-session kinds (0x0002, 0x0007, 0x0009, 0x0102,
#     0x0310-0x0312): `sealed`, HPKE Base to the recipient identity X25519
#     plus the sender's identity signature over
#     `DM_PRESESSION_SIG || recipient || typeId || enc || ciphertext`;
#   - every other kind: `ratchet`.
# Every ratchet and PreKeyInit plaintext is a `DmContent`; every sealed
# plaintext is a `SealedContent`. Their `payload` is the kind's body below
# (empty for FriendReject, Unfriended, UnfriendedAck and
# SessionResetAccept). Ordinals are never reused (§5).

struct DmFrame {
  typeId @0 :UInt16;
  sender @1 :Data;                      # 32-byte Ed25519 identity (identity == address)
  body :union {
    ratchet @2 :RatchetBody;            # {header {dh, pn, n}, ciphertext}; AD binds both identities
    preKeyInit @3 :PreKeyInitBody;      # PQXDH init + header + encrypted first payload
    sealed @4 :SealedBody;              # {enc, ciphertext, signature} (pre-session kinds)
  }
}

# The Double Ratchet header (rev. 1 §3.2).
struct RatchetHeader {
  dh @0 :Data;                          # the sender's ratchet public key (32)
  pn @1 :UInt32;                        # length of the previous sending chain
  n @2 :UInt32;                         # message number in this chain
}

struct RatchetBody {
  header @0 :RatchetHeader;
  ciphertext @1 :Data;                  # AEAD ciphertext and tag of a DmContent
}

# The initiator's first message (PQXDH rev. 3 §3.3). IK_A is the frame's
# `sender`.
struct PreKeyInitBody {
  baseKey @0 :Data;                     # EK_A public (32): the session's base key
  signedPrekeyId @1 :UInt32;
  oneTimePrekey :union {
    none @2 :Void;                      # DH4 did not run
    id @3 :UInt32;
  }
  pqPrekey :union {
    oneTime @4 :UInt32;
    lastResort @5 :UInt32;
  }
  kemCiphertext @6 :Data;               # ML-KEM-768 CT (1088)
  firstTypeId @7 :UInt16;               # the kind of the encrypted first payload
  message @8 :RatchetBody;              # the first DmContent under the new session
}

struct SealedBody {
  enc @0 :Data;                         # HPKE encapsulated key (32)
  ciphertext @1 :Data;                  # HPKE ciphertext of a SealedContent
  signature @2 :Data;                   # sender identity Ed25519 signature (64)
}

struct DmContent { contentHint @0 :ContentHint; sentAtMs @1 :UInt64; payload @2 :Data; }  # the plaintext
enum ContentHint { default @0; resendable @1; implicit @2; }

# The plaintext of a sealed frame: the replay guard keeps (sender, nonce)
# within the freshness window.
struct SealedContent {
  nonce @0 :Data;                       # 16 random bytes
  sentAtMs @1 :UInt64;
  payload @2 :Data;
}

# --- Friendship and session (0x00xx, 0x01xx) ---

# 0x0002, sealed. The bundle field is renamed `obsoletePrekeyBundle` at E2.4.
struct FriendRequest {
  displayName @0 :Text;
  message @1 :Text;
  prekeyBundle @2 :Data;                # PreKeyBundle (identity.capnp), packed
  profileDhtKey @3 :Text;
  routeBlob @4 :Data;
  mailboxDhtKey @5 :Text;
  inviteId @6 :Text;                    # empty when the request answers no invite
}

# 0x0101, the first payload of a PreKeyInit: the session it opens is the
# acceptance, so it carries only where to reach the acceptor.
struct FriendAccept {
  profileDhtKey @0 :Text;
  routeBlob @1 :Data;
  mailboxDhtKey @2 :Text;
}

# 0x0007, sealed. Contents defined by E2.4 (the bundle field is renamed
# `obsoletePrekeyBundle` there).
struct SessionResetRequest {
  prekeyBundle @0 :Data;
}

# 0x0009, sealed.
struct SessionResetDecline {
  reason @0 :Text;
}

# 0x0105.
struct ProfileKeyRotated {
  newProfileDhtKey @0 :Text;
}

# 0x0108, over the old session; both signatures over
# `IDENTITY_ROTATED || old || new || recipient`.
struct IdentityRotated {
  newIdentity @0 :Data;                 # 32
  sigOld @1 :Data;                      # 64, by the old identity
  sigNew @2 :Data;                      # 64, by the new identity
}

# --- Content (0x01xx, 0x04xx) ---

# 0x0100.
struct DmText {
  clientId @0 :Data;                    # UUID v7 (16), minted at send
  body @1 :Text;
  replyTo @2 :Data;                     # the parent's clientId (16), empty when not a reply
  expireSeconds @3 :UInt32;             # 0 = no timer
}

# 0x0110.
struct DmEdit {
  targetClientId @0 :Data;              # 16
  body @1 :Text;
  editedAt @2 :UInt64;                  # milliseconds
}

# 0x0111.
struct DmDelete {
  targetClientId @0 :Data;              # 16
}

# 0x0112.
struct DmReaction {
  targetClientId @0 :Data;              # 16
  emoji @1 :Text;
  add @2 :Bool;
}

# 0x0401.
struct Typing {
  typing @0 :Bool;
}

# --- Strand relay, status probes and push relay (moved unchanged) ---

# 0x0003.
struct RelayEnvelope {
  targetPubkey @0 :Text;
  innerPayload @1 :Data;
}

# 0x0130.
struct RelayOffer {
  relayRouteBlob @0 :Data;
  relayPseudonym @1 :Text;
}

# 0x0131.
struct RelayWithdraw {
  relayPseudonym @0 :Text;
}

# 0x0132.
struct RelayOfferAck {
  ok @0 :Bool;
  reason @1 :Text;
}

# 0x0140.
struct StatusRequest {
  targetPubkey @0 :Text;
}

# 0x0141.
struct StatusResponse {
  targetPubkey @0 :Text;
  status @1 :Text;
  statusMessage @2 :Text;               # empty when none
  lastSeen @3 :UInt64;
  routeBlob @4 :Data;
}

# 0x0310, sealed.
struct RegisterPushRelay {
  devicePushToken @0 :Text;
  platform @1 :Text;
  recordKeys @2 :List(Text);
}

# 0x0311, sealed.
struct UnregisterPushRelay {
  devicePushToken @0 :Text;
}

# 0x0312, sealed.
struct WakeNotify {
  ts @0 :UInt64;
}

# --- Calls (0x02xx, moved unchanged; E4.2 replaces them) ---

# 0x0200.
struct CallInvite {
  callId @0 :Text;
  offerKind @1 :UInt8;
  initiatorPubkey @2 :Text;
  initiatorX25519Pub @3 :Data;
  expiresAtMs @4 :UInt64;
  videoDecodeCodecs @5 :List(Text);
}

# 0x0201.
struct CallRinging {
  callId @0 :Text;
}

# 0x0202.
struct CallAccept {
  callId @0 :Text;
  acceptorX25519Pub @1 :Data;
  videoDecodeCodecs @2 :List(Text);
}

# 0x0203 CallDecline, 0x0204 CallEnd, 0x0212 GroupCallDecline.
struct CallClosed {
  callId @0 :Text;
  reason @1 :Text;
}

# 0x0205.
struct CallMediaState {
  callId @0 :Text;
  audio @1 :Bool;
  video @2 :Bool;
  screen @3 :Bool;
  timestampMs @4 :UInt64;
}

# 0x0206.
struct CallReaction {
  callId @0 :Text;
  emoji @1 :Text;
  timestampMs @2 :UInt64;
}

# 0x0210.
struct GroupCallOffer {
  callId @0 :Text;
  offerKind @1 :UInt8;
  initiatorPubkey @2 :Text;
  initiatorX25519Pub @3 :Data;
  participants @4 :List(Text);
  wrappedCallKey @5 :Data;
  expiresAtMs @6 :UInt64;
}

# 0x0211 GroupCallAccept, 0x0213 GroupCallParticipantJoined.
struct GroupCallMember {
  callId @0 :Text;
  pubkey @1 :Text;
}

# 0x0214.
struct GroupCallParticipantLeft {
  callId @0 :Text;
  participantPubkey @1 :Text;
  reason @2 :Text;
}

# 0x0220, until E4.3.8.
struct DmVideoFragment {
  streamId @0 :Data;                    # 16
  frameSeq @1 :UInt32;
  fragmentIndex @2 :UInt16;
  fragmentCount @3 :UInt16;
  keyframe @4 :Bool;
  codec @5 :Text;
  timestamp @6 :UInt32;
  chunk @7 :Data;
}
