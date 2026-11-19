# Architecture §29 — community gossip-mesh envelope.
#
# Outer wrapper: `SignedEnvelope` carries the Ed25519 signature over the
# `payload` bytes, the gossip TTL hop counter, the sender's hex-encoded
# community pseudonym, and the community id. Relays verify the
# signature, decrement TTL, and forward bytes intact — including for
# unknown variants (forward-compat policy: matches libp2p gossipsub /
# Briar BSP / Matrix federation behaviour).
#
# Inner union: `CommunityEnvelope` (5 outer arms) wraps the typed
# payload variants. The `Control` arm holds a `ControlPayload` (67
# typed variants) that covers all non-chat operations.
#
# Wire-evolution rules: append-only ordinals (@N). Never reorder, never
# reuse, never remove. Old peers ignore unknown union discriminants
# (Cap'n Proto `which()` returns `Err(NotInSchema)`).

@0x868847d85d782d8c;

using import "community_event.capnp".EventInfo;
using import "community_thread.capnp".ThreadInfo;
using import "community_game_server.capnp".GameServerInfo;
using import "governance_op.capnp".OpRef;
using import "community_governance.capnp".GovernanceEntry;

# ── Outer wrapper ─────────────────────────────────────────────────────

# Architecture §29.2 — signed gossip envelope. The signature covers
# `payload` bytes exactly; relays preserve those bytes intact even when
# they don't understand the inner variant.
struct SignedEnvelope @0xe0d4eb16e9bf1c2e {
    # DHT key of the community's governance record (string form).
    communityId      @0 :Text;
    # Hex-encoded Ed25519 public key of the sender's community pseudonym.
    senderPseudonym  @1 :Text;
    # Cap'n Proto-encoded `CommunityEnvelope`. Treated as opaque bytes
    # for signature/dedup; decoded lazily after verification.
    payload          @2 :Data;
    # 64-byte Ed25519 signature over `payload`.
    signature        @3 :Data;
    # Hop TTL — starts at 5, decremented on each forward.
    ttl              @4 :UInt8;
}

# ── Outer union (four live arms) ─────────────────────────────────────────────────

struct CommunityEnvelope @0xe1d4eb16e9bf1c2e {
    union {
        # @0 was `messageNotification` (plan E3.3 slice 8): a post's hint is
        # the author's `HeadAdvanced` for `M`/4, a control payload.
        obsolete0           @0 :Void;
        control             @1 :ControlPayload;
        presenceUpdate      @2 :PresenceUpdate;
        typingIndicator     @3 :TypingIndicator;
        watchRelay          @4 :WatchRelay;
    }
}

# Architecture §11 — presence broadcast. Carries pseudonym + status +
# optional rich-presence game info + optional route blob.
struct PresenceUpdate @0xe3d4eb16e9bf1c2e {
    pseudonymKey     @0 :Text;
    status           @1 :Text;
    hasGameInfo      @2 :Bool;
    gameInfo         @3 :PresenceGameInfo;
    hasRouteBlob     @4 :Bool;
    routeBlob        @5 :Data;
}

struct PresenceGameInfo @0xe4d4eb16e9bf1c2e {
    gameName         @0 :Text;
    hasGameId        @1 :Bool;
    gameId           @2 :UInt32;
    hasElapsedSecs   @3 :Bool;
    elapsedSeconds   @4 :UInt32;
    hasServerAddress @5 :Bool;
    serverAddress    @6 :Text;
}

struct TypingIndicator @0xe5d4eb16e9bf1c2e {
    channelId        @0 :Text;
    pseudonymKey     @1 :Text;
}

struct WatchRelay @0xe6d4eb16e9bf1c2e {
    recordKey        @0 :Text;
    subkey           @1 :UInt32;
    contentHash      @2 :Text;
    observerPseudonym @3 :Text;
}

# ── Sub-types referenced by ControlPayload variants ───────────────────

struct VoiceRosterEntry @0xe8d4eb16e9bf1c2e {
    pseudonymKey     @0 :Text;
    routeBlob        @1 :Data;
    muted            @2 :Bool;
    deafened         @3 :Bool;
    # Display name as known to the roster broadcaster. Empty = unknown.
    displayName      @4 :Text;
    # Plan C7.29: the participant's per-session media key (X25519) and its
    # pseudonym signature over it, relayed as the participant announced it.
    sessionKey       @5 :Data;
    sessionKeySig    @6 :Data;
}

struct OnboardingAnswer @0xe9d4eb16e9bf1c2e {
    questionId       @0 :Text;
    selectedOptions  @1 :List(Text);
}

# ── Per-variant payload structs (one per ControlPayload arm) ──────────
#
# Each is named `<Variant>Payload` so the union discriminant in
# `ControlPayload` reads cleanly. Optional fields use `has<Field>`
# flags. Variant numbering matches the order of declaration in the
# Rust enum at `rekindle-protocol::dht::community::envelope`; the
# union ordinals below are append-only.

struct MemberLeavePayload @0xea0002000000a000 {
    pseudonymKey         @0 :Text;
}

struct MemberJoinedPayload @0xea0005000000a000 {
    pseudonymKey         @0 :Text;
    displayName          @1 :Text;
    roleIds              @2 :List(Data);    # 16-byte role ids
    status               @3 :Text;
    hasRouteBlob         @4 :Bool;
    routeBlob            @5 :Data;
}

struct KickPayload @0xea0007000000a000 {
    targetPseudonym      @0 :Text;
}

struct MEKRotatedPayload @0xea000f000000a000 {
    hasChannelId         @0 :Bool;
    channelId            @1 :Text;
    newGeneration        @2 :UInt64;
    hasRotatorPseudonym  @3 :Bool;
    rotatorPseudonym     @4 :Text;
}

struct RequestMEKPayload @0xea0010000000a000 {
    channelId            @0 :Text;
    neededGeneration     @1 :UInt64;
    requesterPseudonym   @2 :Text;
    # A3/P1.3 — cascade index. Field is appended (Cap'n Proto rule: new
    # fields are zero-default for older peers that don't send it). 0 =
    # deterministic top-rank responder. Requester increments after each
    # 5s timeout to fall through to the next-best candidate.
    cascadeIndex         @3 :UInt32;
}

struct MekTransferPayload @0xea0011000000a000 {
    communityId          @0 :Text;
    hasChannelId         @1 :Bool;
    channelId            @2 :Text;
    generation           @3 :UInt64;
    senderPseudonym      @4 :Text;
    wrappedMek           @5 :Data;
    # Random per push, kept across its retries; the ack echoes it inside
    # its key-confirmation tag (review S7m part 1). It correlates, it does
    # not settle.
    transferId           @6 :Data;
}

# P1.3 — requester → responder ack confirming successful MekTransfer
# ingestion. Sent as the `app_call` reply by the receiver of
# MekTransfer immediately after MEK unwrap succeeds, so the responder
# has positive confirmation that the transfer landed at the app
# layer (not just the network layer). Carries the same identifying
# triple as MekTransfer + the requester's pseudonym so the responder
# can match the ack against in-flight transfers.
struct MekTransferAckPayload @0xea0043000000a000 {
    communityId          @0 :Text;
    hasChannelId         @1 :Bool;
    channelId            @2 :Text;
    generation           @3 :UInt64;
    requesterPseudonym   @4 :Text;
    # The transfer's id, echoed.
    transferId           @5 :Data;
    # The key-confirmation MAC (rekindle_secrets::mek_ack, label
    # rekindle-mek-ack-v1): proves the requester unwrapped exactly the key
    # of this generation (RFC 9420 §12.4.3 confirmation_tag). Required.
    ackTag               @6 :Data;
}

# P4.3 — joiner → admins request to expand the community's Plate Gate
# segments when every slot in the highest existing segment is occupied.
# Sent via gossip when the joiner lacks MANAGE_COMMUNITY; any admin
# receiving this responds by calling `expand_community_segment` which
# emits a `SegmentAdded` governance entry. The joiner watches for the
# new segment via the merged governance state and retries slot claim.
struct RequestSegmentExpansionPayload @0xea0044000000a000 {
    communityId          @0 :Text;
    requesterPseudonym   @1 :Text;
    # Highest segment_index the joiner observed as full when the
    # request was issued — admins use this to dedup concurrent
    # requests (only the first admin to expand wins; later requests
    # see the new segment and no-op).
    fullSegmentIndex     @2 :UInt32;
}

struct OnboardingCompletePayload @0xea0012000000a000 {
    pseudonymKey         @0 :Text;
    roleIds              @1 :List(Data);    # 16-byte role ids
}

struct ChannelOverwriteChangedPayload @0xea0014000000a000 {
    channelId            @0 :Text;
}

struct ReactionAddedPayload @0xea0015000000a000 {
    channelId            @0 :Text;
    messageId            @1 :Text;
    emoji                @2 :Text;
    reactorPseudonym     @3 :Text;
}

struct ReactionRemovedPayload @0xea0016000000a000 {
    channelId            @0 :Text;
    messageId            @1 :Text;
    emoji                @2 :Text;
    reactorPseudonym     @3 :Text;
}

struct MessagePinnedPayload @0xea0017000000a000 {
    channelId            @0 :Text;
    messageId            @1 :Text;
    pinnedBy             @2 :Text;
}

struct MessageUnpinnedPayload @0xea0018000000a000 {
    channelId            @0 :Text;
    messageId            @1 :Text;
}

struct EventCreatedPayload @0xea0019000000a000 {
    event                @0 :EventInfo;
}

struct EventUpdatedPayload @0xea001a000000a000 {
    event                @0 :EventInfo;
}

struct EventDeletedPayload @0xea001b000000a000 {
    eventId              @0 :Text;
}

struct EventRsvpChangedPayload @0xea001c000000a000 {
    eventId              @0 :Text;
    pseudonymKey         @1 :Text;
    status               @2 :Text;
}

struct ThreadCreatedPayload @0xea001d000000a000 {
    thread               @0 :ThreadInfo;
}

struct ThreadMessageReceivedPayload @0xea001e000000a000 {
    threadId             @0 :Text;
    messageId            @1 :Text;
    senderPseudonym      @2 :Text;
    # Obsolete (plan E3.3 slice 8d): a reply's body rides its author's
    # channel outbox (M/4), sealed at its stream position; the notice
    # carries none. Never written, never read.
    obsoleteCiphertext   @3 :Data;
    obsoleteMekGeneration @4 :UInt64;
    timestamp            @5 :UInt64;
    hasReplyToId         @6 :Bool;
    replyToId            @7 :Text;
}

struct ThreadArchivedPayload @0xea001f000000a000 {
    threadId             @0 :Text;
    archived             @1 :Bool;
}

struct GameServerAddedPayload @0xea0020000000a000 {
    server               @0 :GameServerInfo;
}

struct GameServerRemovedPayload @0xea0021000000a000 {
    serverId             @0 :Text;
}

struct SubmitOnboardingAnswersPayload @0xea0022000000a000 {
    answers              @0 :List(OnboardingAnswer);
}

struct EventReminderPayload @0xea0023000000a000 {
    eventId              @0 :Text;
    title                @1 :Text;
    minutesUntilStart    @2 :UInt32;
}

struct RaidAlertPayload @0xea0024000000a000 {
    active               @0 :Bool;
}

struct ChannelLockdownPayload @0xea0025000000a000 {
    locked               @0 :Bool;
}

struct SystemMessagePayload @0xea0026000000a000 {
    body                 @0 :Text;
    timestamp            @1 :UInt64;
}

# The joiner's bootstrap request (plan E3.2): a bare app_call to the
# inviter's current route (its member record's `MemberRoutes`, read at
# join time; plan E3.2 S7c.8). Its authority is the
# joiner's signed `MemberJoined`, which the inviter fetches from the
# joiner's member record by `memberJoined` and verifies before answering.
struct BootstrapRequestPayload @0xea002a000000a000 {
    joinerPseudonym      @0 :Text;
    anchor               @1 :Text;
    memberJoined         @2 :OpRef;
    # The signed op `memberJoined` names, carried so the inviter answers
    # within the app_call reply window without a DHT read (its hash must
    # be `memberJoined.opHash`).
    memberJoinedOp       @3 :Data;
    # The joiner's private route: the inviter delivers the community key
    # to it by `MekTransfer` app_call once the op merges (D16), never in
    # the response (D11).
    joinerRoute          @4 :Data;
}

# The inviter's answer (plan E3.2): where the joiner's walk of the op DAG
# continues. Constant in N: at most 64 frontier heads, under the 32,768 B
# app_call cap. No keys, no messages (D11). The checkpoint hint comes
# before the walk by `checkpointHintRequest`, not here.
struct BootstrapResponseV2Payload @0xea0047000000a000 {
    anchor               @0 :Text;
    frontier             @1 :List(OpRef);
}

struct VoiceJoinPayload @0xea002e000000a000 {
    channelId            @0 :Text;
    routeBlob            @1 :Data;
    # Joiner's self-sovereign display name — identity rides the
    # handshake (SimpleX x.grp.mem.info pattern) so the roster UI
    # never depends on registry-scan timing. Empty = not set.
    displayName          @2 :Text;
    # Plan C7.29: the joiner's per-session media key (X25519) and its
    # pseudonym signature over `domain ‖ community ‖ channel ‖ key`.
    sessionKey           @3 :Data;
    sessionKeySig        @4 :Data;
}

# Seen leg of the three-way join handshake: a present member tells the
# joiner "I received your VoiceJoin, here's me" (directed, ttl=0).
struct VoiceJoinAckPayload @0xea0045000000a000 {
    channelId            @0 :Text;
    # Pseudonym of the joiner being acked (dedup/filter on receive).
    joinerPseudonym      @1 :Text;
    # Acker's display name + route so the ack alone is enough for the
    # joiner to add the acker to its media roster.
    displayName          @2 :Text;
    routeBlob            @3 :Data;
    # Plan C7.29: the acker's per-session media key and its signature.
    sessionKey           @4 :Data;
    sessionKeySig        @5 :Data;
}

# Confirmed leg: the joiner tells channel members "transport-ready —
# start sending me media" (directed, ttl=0). Receivers force a video
# keyframe (RFC 5104 FIR semantics: new member needs a full intra).
struct VoiceJoinConfirmedPayload @0xea0046000000a000 {
    channelId            @0 :Text;
}

# Plans C7.20/C7.22/C7.29: `sender`'s call-media key sealed to
# `recipient`'s per-session key (HPKE Base, bound to channel, recipient
# and index), signed by the sender's pseudonym over
# `domain ‖ channel ‖ recipient ‖ keyIndex ‖ sealed`. Delivered by
# app_call to the recipient's route and answered with VoiceMediaKeyAck;
# never gossiped.
struct VoiceMediaKeyPayload @0xf0582658caa27994 {
    channelId            @0 :Text;
    recipient            @1 :Text;
    keyIndex             @2 :UInt64;
    sealed               @3 :Data;
    communityId          @4 :Text;
    sender               @5 :Text;
    signature            @6 :Data;
}

# Plan C7.20/C7.22: `requester` asks `sender` for its call-media key at
# `keyIndex`, by app_call to the sender's route.
struct VoiceMediaKeyRequestPayload @0xec201d80acd40911 {
    channelId            @0 :Text;
    sender               @1 :Text;
    keyIndex             @2 :UInt64;
    communityId          @3 :Text;
    requester            @4 :Text;
}

# Plan C7.22: the reply to a VoiceMediaKey — `recipient` opened and
# installed `sender`'s key at `keyIndex`.
struct VoiceMediaKeyAckPayload @0xabdce9cd05ea8d37 {
    communityId          @0 :Text;
    channelId            @1 :Text;
    sender               @2 :Text;
    recipient            @3 :Text;
    keyIndex             @4 :UInt64;
}

struct VoiceLeavePayload @0xea002f000000a000 {
    channelId            @0 :Text;
}

struct VoiceModeSwitchPayload @0xea0030000000a000 {
    channelId            @0 :Text;
    mode                 @1 :Text;
    hasHostPseudonym     @2 :Bool;
    hostPseudonym        @3 :Text;
}

struct StageUpdatePayload @0xea0031000000a000 {
    channelId            @0 :Text;
    hasTopic             @1 :Bool;
    topic                @2 :Text;
    speakers             @3 :List(Text);
    moderatorPseudonym   @4 :Text;
    lamport              @5 :UInt64;
}

struct SpeakRequestPayload @0xea0032000000a000 {
    channelId            @0 :Text;
    requesterPseudonym   @1 :Text;
    lamport              @2 :UInt64;
}

struct SpeakResponsePayload @0xea0033000000a000 {
    channelId            @0 :Text;
    requesterPseudonym   @1 :Text;
    granted              @2 :Bool;
    moderatorPseudonym   @3 :Text;
    lamport              @4 :UInt64;
}

struct RequestAttachmentPayload @0xea0034000000a000 {
    channelId            @0 :Text;
    # 16-byte UUID.
    attachmentId         @1 :Data;
    requestedChunks      @2 :List(UInt32);
    requesterPseudonym   @3 :Text;
}

struct AttachmentChunkPayload @0xea0035000000a000 {
    # 16-byte UUID.
    attachmentId         @0 :Data;
    chunkIndex           @1 :UInt32;
    data                 @2 :Data;
    # SHA-256 (32 bytes).
    plaintextHash        @3 :Data;
}

struct MultiAttachmentChunkPayload @0xea0036000000a000 {
    # Self-reference into ControlPayload; capnp permits this.
    chunks               @0 :List(ControlPayload);
}

struct VoiceMutePayload @0xea0037000000a000 {
    channelId            @0 :Text;
    targetPseudonym      @1 :Text;
    muted                @2 :Bool;
}

struct VoiceDeafenPayload @0xea0038000000a000 {
    channelId            @0 :Text;
    targetPseudonym      @1 :Text;
    deafened             @2 :Bool;
}

struct VoiceRosterPayload @0xea0039000000a000 {
    channelId            @0 :Text;
    participants         @1 :List(VoiceRosterEntry);
}

struct SoundboardPlayPayload @0xea003a000000a000 {
    channelId            @0 :Text;
    expressionId         @1 :Text;
    actorPseudonym       @2 :Text;
}

struct VideoFragmentPayload @0xea003b000000a000 {
    channelId            @0 :Text;
    # 16-byte stream id.
    streamId             @1 :Data;
    frameSeq             @2 :UInt32;
    fragIndex            @3 :UInt8;
    fragTotal            @4 :UInt8;
    keyframe             @5 :Bool;
    timestamp            @6 :UInt32;
    payload              @7 :Data;
    signature            @8 :Data;
    # RTP payload-type analog — receivers configure their decoder from
    # this tag. Signature-covered (the codec byte rides the fragment
    # signing bytes).
    codec                @9 :Codec;
    # The sender's media key index (plan C7.20: sender-owned keys) the
    # frame is encrypted under; a receiver without it asks the sender for
    # exactly this index. Signature-covered.
    keyIndex             @10 :UInt64;
}

struct VideoParityFragmentPayload @0xea003c000000a000 {
    channelId            @0 :Text;
    # 16-byte stream id.
    streamId             @1 :Data;
    frameSeq             @2 :UInt32;
    parityIndex          @3 :UInt8;
    parityTotal          @4 :UInt8;
    dataCount            @5 :UInt8;
    frameLen             @6 :UInt32;
    timestamp            @7 :UInt32;
    payload              @8 :Data;
    signature            @9 :Data;
    # Codec of the frame this parity covers — mirrors
    # VideoFragmentPayload.codec; signature-covered.
    codec                @10 :Codec;
    # Mirrors VideoFragmentPayload.keyIndex; signature-covered.
    keyIndex             @11 :UInt64;
}

struct KeyframeRequestPayload @0xea003e000000a000 {
    channelId            @0 :Text;
    # 16-byte stream id.
    streamId             @1 :Data;
}

# Video codec identifier. Mirrors `rekindle_types::video::Codec`.
# Closed enum — no `Other(String)` escape hatch. Adding a codec is a
# deliberate schema break, not a fallback.
enum Codec @0xeb0001000000a000 {
    vp9  @0;
    vp8  @1;
    h264 @2;
}

# VP9 SVC scalability mode. Mirrors `rekindle_types::video::ScalabilityMode`.
enum ScalabilityMode @0xeb0002000000a000 {
    flat @0;
    l1t2 @1;
}

# Pre-release schema break (second re-shape, same precedent as the
# first: memory rule `feedback_no_legacy_compat`, no version-bump
# preserving the old shape). The symmetric `codecsTyped @4` list split
# into direction-specific `encodeCodecs @4` + `decodeCodecs @5`
# (WebView engines are encode/decode asymmetric — Apple WebKit
# guarantees H.264 hw encode but not VP9 encode), which pushes
# `supportedScalabilityModes` to @6. The 64-bit struct ID
# (@0xea0040000000a000) is unchanged because this is the same logical
# payload: any peer attempting to decode the pre-break shape against
# this struct gets a clean schema error, not a silent type-confusion.
# Adding a new field in the future follows append-only discipline from
# @7 onward.
struct MediaCapabilitiesPayload @0xea0040000000a000 {
    channelId                       @0 :Text;
    maxPixelCount                   @1 :UInt32;
    maxFps                          @2 :UInt8;
    supportsOptimizeForLatency      @3 :Bool;
    encodeCodecs                    @4 :List(Codec);
    decodeCodecs                    @5 :List(Codec);
    supportedScalabilityModes       @6 :List(ScalabilityMode);
}

struct TopologyChangePayload @0xea0041000000a000 {
    channelId            @0 :Text;
    # 16-byte stream id.
    streamId             @1 :Data;
    hasRelayHost         @2 :Bool;
    relayHostPseudonym   @3 :Text;
    reason               @4 :Text;
    lamport              @5 :UInt64;
}

struct LinkPreviewPayload @0xea0042000000a000 {
    channelId            @0 :Text;
    messageId            @1 :Text;
    url                  @2 :Text;
    hasTitle             @3 :Bool;
    title                @4 :Text;
    hasDescription       @5 :Bool;
    description          @6 :Text;
    # Never set. Previews carry no image: rendering a peer-chosen image URL
    # would make every reader fetch it and leak their IP. Only ordinals are
    # on the wire, so the rename is compatible.
    obsoleteHasImageUrl  @7 :Bool;
    obsoleteImageUrl     @8 :Text;
    hasSiteName          @9 :Bool;
    siteName             @10 :Text;
    fetchedAt            @11 :UInt64;
}

# ── control-payload union ─────────────────────────────────────────────
#
# Append-only ordinals. New variants get the next number. Removed
# variants become the sentinel `obsolete<N>` arm that decoders skip:
# @61 (frameAck) and @63 (bandwidthEstimate), video's own congestion
# feedback, replaced by the media route's transport feedback (plan
# E4.3.3); @0, @2, @3, @5, @7–@11, @18 and @39–@41 (the unsigned
# membership and moderation notices, the legacy join answer, the
# keypair grants and governanceUpdated), replaced by the signed
# governance ops (plan E3.2 S8); @12 and @13 (messageEdited and
# messageDeleted, the context-free edit seal and the moderator's delete
# notice), replaced by the edit and delete entries in the author's record
# and the readers' apply of the `AdminDelete` op (plan E3.3).

struct ControlPayload @0xeaffffff00000001 {
    union {
        obsolete0                @0  :Void;
        memberLeave              @1  :MemberLeavePayload;
        obsolete2                @2  :Void;
        obsolete3                @3  :Void;
        memberJoined             @4  :MemberJoinedPayload;
        obsolete5                @5  :Void;
        kick                     @6  :KickPayload;
        obsolete7                @7  :Void;
        obsolete8                @8  :Void;
        obsolete9                @9  :Void;
        obsolete10               @10 :Void;
        obsolete11               @11 :Void;
        obsolete12               @12 :Void;
        obsolete13               @13 :Void;
        mekRotated               @14 :MEKRotatedPayload;
        requestMek               @15 :RequestMEKPayload;
        mekTransfer              @16 :MekTransferPayload;
        onboardingComplete       @17 :OnboardingCompletePayload;
        obsolete18               @18 :Void;
        channelOverwriteChanged  @19 :ChannelOverwriteChangedPayload;
        reactionAdded            @20 :ReactionAddedPayload;
        reactionRemoved          @21 :ReactionRemovedPayload;
        messagePinned            @22 :MessagePinnedPayload;
        messageUnpinned          @23 :MessageUnpinnedPayload;
        eventCreated             @24 :EventCreatedPayload;
        eventUpdated             @25 :EventUpdatedPayload;
        eventDeleted             @26 :EventDeletedPayload;
        eventRsvpChanged         @27 :EventRsvpChangedPayload;
        threadCreated            @28 :ThreadCreatedPayload;
        threadMessageReceived    @29 :ThreadMessageReceivedPayload;
        threadArchived           @30 :ThreadArchivedPayload;
        gameServerAdded          @31 :GameServerAddedPayload;
        gameServerRemoved        @32 :GameServerRemovedPayload;
        submitOnboardingAnswers  @33 :SubmitOnboardingAnswersPayload;
        eventReminder            @34 :EventReminderPayload;
        # Unit variant — no payload bytes; presence of ordinal is the
        # signal.
        obsolete35               @35 :Void;
        raidAlert                @36 :RaidAlertPayload;
        channelLockdown          @37 :ChannelLockdownPayload;
        systemMessage            @38 :SystemMessagePayload;
        obsolete39               @39 :Void;
        obsolete40               @40 :Void;
        obsolete41               @41 :Void;
        bootstrapRequest         @42 :BootstrapRequestPayload;
        obsolete43               @43 :Void;
        obsolete44               @44 :Void;
        obsolete45               @45 :Void;
        voiceJoin                @46 :VoiceJoinPayload;
        voiceLeave               @47 :VoiceLeavePayload;
        voiceModeSwitch          @48 :VoiceModeSwitchPayload;
        stageUpdate              @49 :StageUpdatePayload;
        speakRequest             @50 :SpeakRequestPayload;
        speakResponse            @51 :SpeakResponsePayload;
        requestAttachment        @52 :RequestAttachmentPayload;
        attachmentChunk          @53 :AttachmentChunkPayload;
        multiAttachmentChunk     @54 :MultiAttachmentChunkPayload;
        voiceMute                @55 :VoiceMutePayload;
        voiceDeafen              @56 :VoiceDeafenPayload;
        voiceRoster              @57 :VoiceRosterPayload;
        soundboardPlay           @58 :SoundboardPlayPayload;
        videoFragment            @59 :VideoFragmentPayload;
        videoParityFragment      @60 :VideoParityFragmentPayload;
        obsolete61               @61 :Void;
        keyframeRequest          @62 :KeyframeRequestPayload;
        obsolete63               @63 :Void;
        mediaCapabilities        @64 :MediaCapabilitiesPayload;
        topologyChange           @65 :TopologyChangePayload;
        linkPreview              @66 :LinkPreviewPayload;
        mekTransferAck           @67 :MekTransferAckPayload;
        requestSegmentExpansion  @68 :RequestSegmentExpansionPayload;
        voiceJoinAck             @69 :VoiceJoinAckPayload;
        voiceJoinConfirmed       @70 :VoiceJoinConfirmedPayload;
        voiceMediaKey            @71 :VoiceMediaKeyPayload;
        voiceMediaKeyRequest     @72 :VoiceMediaKeyRequestPayload;
        voiceMediaKeyAck         @73 :VoiceMediaKeyAckPayload;
        # The gossip fast path for a signed governance op (plan E3.2): the
        # encoded GovernanceOp (governance_op.capnp) a member published to
        # its M/2; a receiver verifies it and stores it like a fetched op.
        governanceOp             @74 :Data;
        # The inviter's answer to a bootstrapRequest (plan E3.2).
        bootstrapResponseV2      @75 :BootstrapResponseV2Payload;
        # A ring watcher's hint (plan E3.2 S7): an encoded `HeadAdvanced`
        # (member_record.capnp), signed by the relaying member.
        headAdvanced             @76 :Data;
        # A checkpoint state snapshot request and its answer (plan E3.2
        # S7c.3): an encoded `CheckpointStateRequest`, signed by the asking
        # member, and an encoded `CheckpointStateResponse`
        # (community_governance.capnp).
        checkpointStateRequest   @77 :Data;
        checkpointStateResponse  @78 :Data;
        # A checkpoint hint request (plan E3.2 S7c.5): an encoded, signed
        # `CheckpointHintRequest`, and an encoded `CheckpointHintResponse`
        # (community_governance.capnp).
        checkpointHintRequest    @79 :Data;
        checkpointHintResponse   @80 :Data;
        # A partial responder's answer to a bootstrapRequest (plan E3.2
        # S7c.8): the anchor of a community it joined from a hint and has
        # not backfilled; the joiner asks another member (Synapse's
        # UNABLE_DUE_TO_PARTIAL_STATE), never a refusal.
        bootstrapPartial         @81 :Text;
    }
}
