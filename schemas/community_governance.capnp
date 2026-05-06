# Architecture §29.4 — `GovernanceEntry` 34-arm union written by
# members to their SMPL governance subkey. Mirrors the Rust enum at
# `rekindle-types::governance::GovernanceEntry`.
#
# Wire-evolution rules: append-only ordinals (@N). Never reorder,
# never reuse, never remove. Old peers ignore unknown variants per
# the gossip-relay forward-compat policy: relay verifies the Ed25519
# signature, decrements TTL, forwards the bytes, but does not
# dispatch the unknown entry locally.

@0xab046a85150ae9f1;

using import "community_event.capnp".RecurrenceRule;
using import "community_event.capnp".EventLocation;
using import "community_event.capnp".EventStatus;

# Shared sub-types referenced by governance entries.

# 32-byte Ed25519 community pseudonym public key.
struct PseudonymKey @0xb0d4eb16e9bf1c2e {
    bytes  @0 :Data;
}

# 16-byte UUID. Used for ChannelId, RoleId, CategoryId, ThreadId,
# EventId, AttachmentId, MessageId — context-specific in each variant.
struct Uuid16 @0xb1d4eb16e9bf1c2e {
    bytes  @0 :Data;
}

# Lost Cargo file-attachment manifest (architecture §28.9 lines 3233-3244).
# Mirrors `rekindle-types::attachment::AttachmentOffer` and replaces the old
# inline expression bytes — large expression assets are chunked into the
# local file cache and fetched via the existing RequestAttachment /
# AttachmentChunk flow. Embedded in ExpressionAddedEntry so peers learn the
# manifest as soon as governance merges.
struct AttachmentOffer @0xdad4eb16e9bf1c2e {
    attachmentId        @0 :Uuid16;
    filename            @1 :Text;
    mimeType            @2 :Text;
    totalSize           @3 :UInt64;
    chunkCount          @4 :UInt32;
    chunkSize           @5 :UInt32;
    merkleRoot          @6 :Data;
    chunkHashes         @7 :List(Data);
    wrappedFek          @8 :Data;
    fekMekGeneration    @9 :UInt64;
}

struct OnboardingOption @0xb2d4eb16e9bf1c2e {
    optionId         @0 :Text;
    title            @1 :Text;
    hasDescription   @2 :Bool;
    description      @3 :Text;
    hasEmoji         @4 :Bool;
    emoji            @5 :Text;
    rolesToAssign    @6 :List(Uuid16);
    channelsToShow   @7 :List(Uuid16);
}

struct OnboardingQuestion @0xb3d4eb16e9bf1c2e {
    questionId       @0 :Text;
    title            @1 :Text;
    hasDescription   @2 :Bool;
    description      @3 :Text;
    required         @4 :Bool;
    singleSelect     @5 :Bool;
    options          @6 :List(OnboardingOption);
}

struct GuideStep @0xb4d4eb16e9bf1c2e {
    title            @0 :Text;
    description      @1 :Text;
    hasChannelId     @2 :Bool;
    channelId        @3 :Uuid16;
    hasEmoji         @4 :Bool;
    emoji            @5 :Text;
}

struct WelcomeChannel @0xb5d4eb16e9bf1c2e {
    channelId        @0 :Uuid16;
    description      @1 :Text;
    hasEmoji         @2 :Bool;
    emoji            @3 :Text;
}

# Variant payloads for `GovernanceEntry`. Each carries `lamport` for
# CRDT ordering plus the per-variant fields. Optional sub-fields use
# `has<Field>` flags.

struct ChannelCreatedEntry @0xb6d4eb16e9bf1c2e {
    channelId        @0 :Uuid16;
    name             @1 :Text;
    # `text` / `voice` / `announcement` / `forum` / `stage` / `media`.
    channelType      @2 :Text;
    recordKey        @3 :Text;
    hasCategoryId    @4 :Bool;
    categoryId       @5 :Uuid16;
    position         @6 :UInt32;
    lamport          @7 :UInt64;
    # Architecture §10.8 — text-in-voice. When set, this channel is
    # the text companion of `parentVoiceChannelId`; the frontend
    # hides it unless the local member is connected to that voice
    # channel.
    hasParentVoiceChannelId  @8 :Bool;
    parentVoiceChannelId     @9 :Uuid16;
}

struct ChannelArchivedEntry @0xb7d4eb16e9bf1c2e {
    channelId        @0 :Uuid16;
    lamport          @1 :UInt64;
}

struct ChannelUpdatedEntry @0xb8d4eb16e9bf1c2e {
    channelId            @0 :Uuid16;
    hasName              @1 :Bool;
    name                 @2 :Text;
    hasTopic             @3 :Bool;
    topic                @4 :Text;
    hasForumTags         @5 :Bool;
    forumTags            @6 :List(Text);
    hasPosition          @7 :Bool;
    position             @8 :UInt32;
    hasSlowmodeSeconds   @9 :Bool;
    slowmodeSeconds      @10 :UInt32;
    hasNsfw              @11 :Bool;
    nsfw                 @12 :Bool;
    # Tri-state for category move: hasCategoryId=false → no change;
    # hasCategoryId=true && categoryIdPresent=false → remove from category;
    # hasCategoryId=true && categoryIdPresent=true → move to categoryId.
    hasCategoryId        @13 :Bool;
    categoryIdPresent    @14 :Bool;
    categoryId           @15 :Uuid16;
    lamport              @16 :UInt64;
}

struct RoleDefinitionEntry @0xb9d4eb16e9bf1c2e {
    roleId               @0 :Uuid16;
    name                 @1 :Text;
    permissions          @2 :UInt64;
    position             @3 :UInt32;
    color                @4 :UInt32;
    hoist                @5 :Bool;
    mentionable          @6 :Bool;
    selfAssignable       @7 :Bool;
    hasExclusionGroup    @8 :Bool;
    exclusionGroup       @9 :Text;
    lamport              @10 :UInt64;
}

struct RoleAssignmentEntry @0xbad4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    roleId               @1 :Uuid16;
    lamport              @2 :UInt64;
}

struct RoleUnassignmentEntry @0xbbd4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    roleId               @1 :Uuid16;
    lamport              @2 :UInt64;
}

struct BanEntryPayload @0xbcd4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    hasReason            @1 :Bool;
    reason               @2 :Text;
    lamport              @3 :UInt64;
}

struct UnbanEntryPayload @0xbdd4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    lamport              @1 :UInt64;
}

struct TimeoutEntryPayload @0xbed4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    durationSeconds      @1 :UInt64;
    hasReason            @2 :Bool;
    reason               @3 :Text;
    startedAt            @4 :UInt64;
    lamport              @5 :UInt64;
}

struct RemoveTimeoutEntryPayload @0xbfd4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    lamport              @1 :UInt64;
}

struct CommunityMetaEntry @0xc0d4eb16e9bf1c2e {
    hasName              @0 :Bool;
    name                 @1 :Text;
    hasDescription       @2 :Bool;
    description          @3 :Text;
    hasIconHash          @4 :Bool;
    iconHash             @5 :Text;
    hasBannerHash        @6 :Bool;
    bannerHash           @7 :Text;
    lamport              @8 :UInt64;
}

struct CommunityNotificationDefaultEntry @0xc1d4eb16e9bf1c2e {
    # `all` / `mentions` / `nothing`.
    level                @0 :Text;
    lamport              @1 :UInt64;
}

struct MEKGenerationBumpEntry @0xc2d4eb16e9bf1c2e {
    generation           @0 :UInt64;
    triggerDeparted      @1 :PseudonymKey;
    cascadeSkipped       @2 :List(PseudonymKey);
    lamport              @3 :UInt64;
}

struct CategoryCreatedEntry @0xc3d4eb16e9bf1c2e {
    categoryId           @0 :Uuid16;
    name                 @1 :Text;
    position             @2 :UInt32;
    lamport              @3 :UInt64;
}

struct CategoryArchivedEntry @0xc4d4eb16e9bf1c2e {
    categoryId           @0 :Uuid16;
    lamport              @1 :UInt64;
}

struct PermissionOverwriteEntry @0xc5d4eb16e9bf1c2e {
    channelId            @0 :Uuid16;
    # `role` or `member`.
    targetType           @1 :Text;
    # Role ID hex or member pseudonym hex.
    targetId             @2 :Text;
    allow                @3 :UInt64;
    deny                 @4 :UInt64;
    lamport              @5 :UInt64;
}

struct ThreadCreatedEntry @0xc6d4eb16e9bf1c2e {
    threadId             @0 :Uuid16;
    parentChannelId      @1 :Uuid16;
    name                 @2 :Text;
    threadType           @3 :Text;
    hasRecordKey         @4 :Bool;
    recordKey            @5 :Text;
    invited              @6 :List(PseudonymKey);
    hasForumTag          @7 :Bool;
    forumTag             @8 :Text;
    autoArchiveSeconds   @9 :UInt64;
    lamport              @10 :UInt64;
}

struct ThreadArchivedEntry @0xc7d4eb16e9bf1c2e {
    threadId             @0 :Uuid16;
    lamport              @1 :UInt64;
}

struct EventCreatedEntry @0xc8d4eb16e9bf1c2e {
    eventId              @0 :Uuid16;
    name                 @1 :Text;
    hasDescription       @2 :Bool;
    description          @3 :Text;
    startTime            @4 :UInt64;
    hasEndTime           @5 :Bool;
    endTime              @6 :UInt64;
    hasChannelId         @7 :Bool;
    channelId            @8 :Uuid16;
    hasCoverImageRef     @9 :Bool;
    coverImageRef        @10 :Text;
    hasCreatorPseudonym  @11 :Bool;
    creatorPseudonym     @12 :PseudonymKey;
    hasRecurrence        @13 :Bool;
    recurrence           @14 :RecurrenceRule;
    hasLocation          @15 :Bool;
    location             @16 :EventLocation;
    hasStatus            @17 :Bool;
    status               @18 :EventStatus;
    lamport              @19 :UInt64;
}

struct ExpressionAddedEntry @0xc9d4eb16e9bf1c2e {
    expressionId         @0 :Uuid16;
    name                 @1 :Text;
    # `emoji` / `sticker` / `soundboard`.
    kind                 @2 :Text;
    contentHash          @3 :Text;
    # Deprecated: assets >32 KiB cannot fit in a single SMPL subkey.
    # Architecture §18.4 + §28.9 line 3286 specifies Lost Cargo distribution
    # for expression assets; see `attachment` (@17/@18) below. Encoders MUST
    # always set `hasInlineData = false`; decoders ignore the bytes if both
    # are present and prefer `attachment`.
    hasInlineData        @4 :Bool;
    inlineData           @5 :Data;
    animated             @6 :Bool;
    tags                 @7 :List(Text);
    hasSoundMeta         @8 :Bool;
    soundMeta            @9 :SoundboardMeta;
    hasCreatorPseudonym  @10 :Bool;
    creatorPseudonym     @11 :PseudonymKey;
    hasCreatedAt         @12 :Bool;
    createdAt            @13 :UInt64;
    hasAvailableToPeers  @14 :Bool;
    availableToPeers     @15 :Bool;
    lamport              @16 :UInt64;
    # Architecture §18.4 — Lost Cargo manifest. Required for any new
    # expression upload; receivers fetch the asset bytes via the
    # existing RequestAttachment / AttachmentChunk handlers.
    hasAttachment        @17 :Bool;
    attachment           @18 :AttachmentOffer;
}

struct SoundboardMeta @0xcad4eb16e9bf1c2e {
    durationSeconds      @0 :Float32;
    volume               @1 :Float32;
    hasEmoji             @2 :Bool;
    emoji                @3 :Text;
}

struct ExpressionRemovedEntry @0xcbd4eb16e9bf1c2e {
    expressionId         @0 :Uuid16;
    lamport              @1 :UInt64;
}

struct EventArchivedEntry @0xccd4eb16e9bf1c2e {
    eventId              @0 :Uuid16;
    lamport              @1 :UInt64;
}

struct OnboardingConfigEntry @0xcdd4eb16e9bf1c2e {
    enabled              @0 :Bool;
    # `default` / `guided` / `gated`.
    mode                 @1 :Text;
    defaultChannels      @2 :List(Uuid16);
    questions            @3 :List(OnboardingQuestion);
    hasWelcomeMessage    @4 :Bool;
    welcomeMessage       @5 :Text;
    guideSteps           @6 :List(GuideStep);
    lamport              @7 :UInt64;
}

struct WelcomeScreenEntry @0xced4eb16e9bf1c2e {
    description          @0 :Text;
    channels             @1 :List(WelcomeChannel);
    lamport              @2 :UInt64;
}

struct AdminDeleteEntry @0xcfd4eb16e9bf1c2e {
    messageId            @0 :Uuid16;
    channelId            @1 :Uuid16;
    hasReason            @2 :Bool;
    reason               @3 :Text;
    lamport              @4 :UInt64;
}

struct ChannelSegmentLinkedEntry @0xd0d4eb16e9bf1c2e {
    channelId            @0 :Uuid16;
    segmentIndex         @1 :UInt32;
    recordKey            @2 :Text;
    lamport              @3 :UInt64;
}

struct SegmentAddedEntry @0xd1d4eb16e9bf1c2e {
    segmentIndex         @0 :UInt32;
    registryKey          @1 :Text;
    governanceKey        @2 :Text;
    slotRangeStart       @3 :UInt32;
    slotRangeEnd         @4 :UInt32;
    lamport              @5 :UInt64;
}

struct AutoModRuleEntry @0xd2d4eb16e9bf1c2f {
    ruleId               @0 :Uuid16;
    name                 @1 :Text;
    enabled              @2 :Bool;
    triggerJson          @3 :Text;
    # `block_locally` / `blur_content` / `alert_moderators`.
    action               @4 :Text;
    lamport              @5 :UInt64;
}

struct RoleArchivedEntry @0xd3d4eb16e9bf1c2e {
    roleId               @0 :Uuid16;
    lamport              @1 :UInt64;
}

struct CategoryUpdatedEntry @0xd4d4eb16e9bf1c2e {
    categoryId           @0 :Uuid16;
    hasName              @1 :Bool;
    name                 @2 :Text;
    hasPosition          @3 :Bool;
    position             @4 :UInt32;
    lamport              @5 :UInt64;
}

struct InviteCreatedEntry @0xd5d4eb16e9bf1c2e {
    inviteId             @0 :Uuid16;
    codeHash             @1 :Text;
    maxUses              @2 :UInt32;
    hasExpiresAt         @3 :Bool;
    expiresAt            @4 :UInt64;
    encryptedSecrets     @5 :Text;
    lamport              @6 :UInt64;
}

struct InviteRevokedEntry @0xd6d4eb16e9bf1c2e {
    inviteId             @0 :Uuid16;
    lamport              @1 :UInt64;
}

struct AttachmentPinnedEntry @0xd7d4eb16e9bf1c2e {
    attachmentId         @0 :Uuid16;
    pinned               @1 :Bool;
    lamport              @2 :UInt64;
}

struct CommunityPolicyEntry @0xd8d4eb16e9bf1c2e {
    hasPolicyText        @0 :Bool;
    policyText           @1 :Text;
    maxJoinsPerInterval  @2 :UInt32;
    joinIntervalSeconds  @3 :UInt32;
    lamport              @4 :UInt64;
}

# Top-level governance entry union — 34 arms, append-only.
struct GovernanceEntry @0xd9d4eb16e9bf1c2e {
    union {
        channelCreated                @0  :ChannelCreatedEntry;
        channelArchived               @1  :ChannelArchivedEntry;
        channelUpdated                @2  :ChannelUpdatedEntry;
        roleDefinition                @3  :RoleDefinitionEntry;
        roleAssignment                @4  :RoleAssignmentEntry;
        roleUnassignment              @5  :RoleUnassignmentEntry;
        banEntry                      @6  :BanEntryPayload;
        unbanEntry                    @7  :UnbanEntryPayload;
        timeoutEntry                  @8  :TimeoutEntryPayload;
        removeTimeoutEntry            @9  :RemoveTimeoutEntryPayload;
        communityMeta                 @10 :CommunityMetaEntry;
        communityNotificationDefault  @11 :CommunityNotificationDefaultEntry;
        mekGenerationBump             @12 :MEKGenerationBumpEntry;
        categoryCreated               @13 :CategoryCreatedEntry;
        categoryArchived              @14 :CategoryArchivedEntry;
        permissionOverwrite           @15 :PermissionOverwriteEntry;
        threadCreated                 @16 :ThreadCreatedEntry;
        threadArchived                @17 :ThreadArchivedEntry;
        eventCreated                  @18 :EventCreatedEntry;
        expressionAdded               @19 :ExpressionAddedEntry;
        expressionRemoved             @20 :ExpressionRemovedEntry;
        eventArchived                 @21 :EventArchivedEntry;
        onboardingConfig              @22 :OnboardingConfigEntry;
        welcomeScreen                 @23 :WelcomeScreenEntry;
        adminDelete                   @24 :AdminDeleteEntry;
        channelSegmentLinked          @25 :ChannelSegmentLinkedEntry;
        segmentAdded                  @26 :SegmentAddedEntry;
        autoModRule                   @27 :AutoModRuleEntry;
        roleArchived                  @28 :RoleArchivedEntry;
        categoryUpdated               @29 :CategoryUpdatedEntry;
        inviteCreated                 @30 :InviteCreatedEntry;
        inviteRevoked                 @31 :InviteRevokedEntry;
        attachmentPinned              @32 :AttachmentPinnedEntry;
        communityPolicy               @33 :CommunityPolicyEntry;
    }
}
