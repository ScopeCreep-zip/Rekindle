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
using import "community_member.capnp".MemberInfo;
using import "community_mek.capnp".ChannelMekDelivery;
using import "community_message.capnp".BootstrapChannelMessages;
using import "community_message.capnp".SyncedMessage;
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

# ── Outer 5-arm union ─────────────────────────────────────────────────

struct CommunityEnvelope @0xe1d4eb16e9bf1c2e {
    union {
        messageNotification @0 :MessageNotification;
        control             @1 :ControlPayload;
        presenceUpdate      @2 :PresenceUpdate;
        typingIndicator     @3 :TypingIndicator;
        watchRelay          @4 :WatchRelay;
    }
}

# Architecture §13 — message-write notification carried over gossip.
# Receivers fetch the actual MEK-encrypted ciphertext from the sender's
# SMPL subkey via `get_dht_value`.
struct MessageNotification @0xe2d4eb16e9bf1c2e {
    channelId        @0 :Text;
    messageId        @1 :Text;
    authorPseudonym  @2 :Text;
    subkeyIndex      @3 :UInt32;
    lamportTs        @4 :UInt64;
    sequence         @5 :UInt64;
    contentHash      @6 :Text;
    timestamp        @7 :UInt64;
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

struct MemberSummary @0xe7d4eb16e9bf1c2e {
    pseudonymKey         @0 :Text;
    displayName          @1 :Text;
    roleIds              @2 :List(UInt32);
    joinedAt             @3 :UInt64;
    subkeyIndex          @4 :UInt32;
    onboardingComplete   @5 :Bool;
    hasTimeoutUntil      @6 :Bool;
    timeoutUntil         @7 :UInt64;
}

struct VoiceRosterEntry @0xe8d4eb16e9bf1c2e {
    pseudonymKey     @0 :Text;
    routeBlob        @1 :Data;
    muted            @2 :Bool;
    deafened         @3 :Bool;
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

struct MemberJoinRequestPayload @0xea0001000000a000 {
    pseudonymKey         @0 :Text;
    displayName          @1 :Text;
    hasInviteCode        @2 :Bool;
    inviteCode           @3 :Text;
    hasRouteBlob         @4 :Bool;
    routeBlob            @5 :Data;
    hasPrekeyBundle      @6 :Bool;
    prekeyBundle         @7 :Data;
    hasClaimedSubkey     @8 :Bool;
    claimedSubkeyIndex   @9 :UInt32;
}

struct MemberLeavePayload @0xea0002000000a000 {
    pseudonymKey         @0 :Text;
}

struct JoinAcceptedPayload @0xea0003000000a000 {
    mekEncrypted          @0 :Data;
    mekGeneration         @1 :UInt64;
    members               @2 :List(MemberSummary);
    hasMemberRegistryKey  @3 :Bool;
    memberRegistryKey     @4 :Text;
    hasSlotIndex          @5 :Bool;
    slotIndex             @6 :UInt32;
    hasWrappedSlotSeed    @7 :Bool;
    wrappedSlotSeed       @8 :Data;
}

struct JoinRejectedPayload @0xea0004000000a000 {
    reason               @0 :Text;
}

struct MemberJoinedPayload @0xea0005000000a000 {
    pseudonymKey         @0 :Text;
    displayName          @1 :Text;
    roleIds              @2 :List(UInt32);
    status               @3 :Text;
    hasRouteBlob         @4 :Bool;
    routeBlob            @5 :Data;
}

struct MemberRemovedPayload @0xea0006000000a000 {
    pseudonymKey         @0 :Text;
}

struct KickPayload @0xea0007000000a000 {
    targetPseudonym      @0 :Text;
}

struct BanPayload @0xea0008000000a000 {
    targetPseudonym      @0 :Text;
}

struct UnbanPayload @0xea0009000000a000 {
    targetPseudonym      @0 :Text;
}

struct TimeoutMemberPayload @0xea000a000000a000 {
    targetPseudonym      @0 :Text;
    durationSeconds      @1 :UInt64;
    hasReason            @2 :Bool;
    reason               @3 :Text;
}

struct RemoveTimeoutPayload @0xea000b000000a000 {
    targetPseudonym      @0 :Text;
}

struct MemberTimedOutPayload @0xea000c000000a000 {
    pseudonymKey         @0 :Text;
    hasTimeoutUntil      @1 :Bool;
    timeoutUntil         @2 :UInt64;
}

struct MessageEditedPayload @0xea000d000000a000 {
    channelId            @0 :Text;
    messageId            @1 :Text;
    newCiphertext        @2 :Data;
    mekGeneration        @3 :UInt64;
    editedAt             @4 :UInt64;
}

struct MessageDeletedPayload @0xea000e000000a000 {
    channelId            @0 :Text;
    messageId            @1 :Text;
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
}

struct OnboardingCompletePayload @0xea0012000000a000 {
    pseudonymKey         @0 :Text;
    roleIds              @1 :List(UInt32);
}

struct MemberRolesChangedPayload @0xea0013000000a000 {
    pseudonymKey         @0 :Text;
    roleIds              @1 :List(UInt32);
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
    ciphertext           @3 :Data;
    mekGeneration        @4 :UInt64;
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

# `KickedNotification` — unit variant (no payload).

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

struct AdminKeypairGrantPayload @0xea0027000000a000 {
    wrappedOwnerKeypair  @0 :Data;
    wrappedSlotSeed      @1 :Data;
}

struct SlotKeypairGrantPayload @0xea0028000000a000 {
    slotIndex            @0 :UInt32;
    segmentIndex         @1 :UInt32;
    wrappedSlotKeypair   @2 :Data;
}

struct GovernanceUpdatedPayload @0xea0029000000a000 {
    governanceKey        @0 :Text;
    subkeyIndex          @1 :UInt32;
    lamportTs            @2 :UInt64;
}

struct BootstrapRequestPayload @0xea002a000000a000 {
    joinerPseudonym      @0 :Text;
    governanceKey        @1 :Text;
}

struct BootstrapResponsePayload @0xea002b000000a000 {
    governanceEntries     @0 :List(GovernanceEntry);
    memberList            @1 :List(MemberInfo);
    channelMeks           @2 :List(ChannelMekDelivery);
    recentMessages        @3 :List(BootstrapChannelMessages);
    wrappedOwnerKeypair   @4 :Data;
}

struct SyncRequestPayload @0xea002c000000a000 {
    channelId            @0 :Text;
    sinceTimestamp       @1 :UInt64;
}

struct SyncResponsePayload @0xea002d000000a000 {
    channelId            @0 :Text;
    messages             @1 :List(SyncedMessage);
}

struct VoiceJoinPayload @0xea002e000000a000 {
    channelId            @0 :Text;
    routeBlob            @1 :Data;
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
}

struct FrameAckPayload @0xea003d000000a000 {
    channelId            @0 :Text;
    # 16-byte stream id.
    streamId             @1 :Data;
    lastFrameSeq         @2 :UInt32;
    kbps                 @3 :UInt32;
    lossQ8               @4 :UInt8;
}

struct KeyframeRequestPayload @0xea003e000000a000 {
    channelId            @0 :Text;
    # 16-byte stream id.
    streamId             @1 :Data;
}

struct BandwidthEstimatePayload @0xea003f000000a000 {
    channelId            @0 :Text;
    kbps                 @1 :UInt32;
    windowSecs           @2 :UInt8;
    lossQ8               @3 :UInt8;
}

struct MediaCapabilitiesPayload @0xea0040000000a000 {
    channelId            @0 :Text;
    maxPixelCount        @1 :UInt32;
    maxFps               @2 :UInt8;
    codecs               @3 :List(Text);
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
    hasImageUrl          @7 :Bool;
    imageUrl             @8 :Text;
    hasSiteName          @9 :Bool;
    siteName             @10 :Text;
    fetchedAt            @11 :UInt64;
}

# ── 67-arm union ──────────────────────────────────────────────────────
#
# Append-only ordinals. New variants get the next number. Removed
# variants (none yet) become the sentinel `obsolete<N>` arm that
# decoders skip.

struct ControlPayload @0xeaffffff00000001 {
    union {
        memberJoinRequest        @0  :MemberJoinRequestPayload;
        memberLeave              @1  :MemberLeavePayload;
        joinAccepted             @2  :JoinAcceptedPayload;
        joinRejected             @3  :JoinRejectedPayload;
        memberJoined             @4  :MemberJoinedPayload;
        memberRemoved            @5  :MemberRemovedPayload;
        kick                     @6  :KickPayload;
        ban                      @7  :BanPayload;
        unban                    @8  :UnbanPayload;
        timeoutMember            @9  :TimeoutMemberPayload;
        removeTimeout            @10 :RemoveTimeoutPayload;
        memberTimedOut           @11 :MemberTimedOutPayload;
        messageEdited            @12 :MessageEditedPayload;
        messageDeleted           @13 :MessageDeletedPayload;
        mekRotated               @14 :MEKRotatedPayload;
        requestMek               @15 :RequestMEKPayload;
        mekTransfer              @16 :MekTransferPayload;
        onboardingComplete       @17 :OnboardingCompletePayload;
        memberRolesChanged       @18 :MemberRolesChangedPayload;
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
        kickedNotification       @35 :Void;
        raidAlert                @36 :RaidAlertPayload;
        channelLockdown          @37 :ChannelLockdownPayload;
        systemMessage            @38 :SystemMessagePayload;
        adminKeypairGrant        @39 :AdminKeypairGrantPayload;
        slotKeypairGrant         @40 :SlotKeypairGrantPayload;
        governanceUpdated        @41 :GovernanceUpdatedPayload;
        bootstrapRequest         @42 :BootstrapRequestPayload;
        bootstrapResponse        @43 :BootstrapResponsePayload;
        syncRequest              @44 :SyncRequestPayload;
        syncResponse             @45 :SyncResponsePayload;
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
        frameAck                 @61 :FrameAckPayload;
        keyframeRequest          @62 :KeyframeRequestPayload;
        bandwidthEstimate        @63 :BandwidthEstimatePayload;
        mediaCapabilities        @64 :MediaCapabilitiesPayload;
        topologyChange           @65 :TopologyChangePayload;
        linkPreview              @66 :LinkPreviewPayload;
    }
}
