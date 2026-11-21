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

using Op = import "governance_op.capnp";
using import "community_event.capnp".RecurrenceRule;
using import "community_event.capnp".EventLocation;
using import "community_event.capnp".EventStatus;
using import "community_event.capnp".RsvpStatus;

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

# Variant payloads for `GovernanceEntry`. The per-variant `lamport`
# fields are retired (`obsoleteLamport`, never read): an op's position
# comes from its signed deps (plan E3.2). Optional sub-fields use
# `has<Field>` flags.

struct ChannelCreatedEntry @0xb6d4eb16e9bf1c2e {
    channelId        @0 :Uuid16;
    name             @1 :Text;
    # `text` / `voice` / `announcement` / `forum` / `stage` / `media`.
    channelType      @2 :Text;
    # Obsolete (plan E3.3): a channel has no record; posts ride `M`/4.
    obsoleteRecordKey @3 :Text;
    hasCategoryId    @4 :Bool;
    categoryId       @5 :Uuid16;
    position         @6 :UInt32;
    obsoleteLamport          @7 :UInt64;
    # Architecture §10.8 — text-in-voice. When set, this channel is
    # the text companion of `parentVoiceChannelId`; the frontend
    # hides it unless the local member is connected to that voice
    # channel.
    hasParentVoiceChannelId  @8 :Bool;
    parentVoiceChannelId     @9 :Uuid16;
}

struct ChannelArchivedEntry @0xb7d4eb16e9bf1c2e {
    channelId        @0 :Uuid16;
    obsoleteLamport          @1 :UInt64;
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
    obsoleteLamport              @16 :UInt64;
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
    obsoleteLamport              @10 :UInt64;
}

struct RoleAssignmentEntry @0xbad4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    roleId               @1 :Uuid16;
    obsoleteLamport              @2 :UInt64;
}

struct RoleUnassignmentEntry @0xbbd4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    roleId               @1 :Uuid16;
    obsoleteLamport              @2 :UInt64;
}

struct BanEntryPayload @0xbcd4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    hasReason            @1 :Bool;
    reason               @2 :Text;
    obsoleteLamport              @3 :UInt64;
    # The number of the target's ops in the ban's causal past (s_k + 1;
    # 0: none): every later op of the target is masked (plan E3.2 S7c.3a).
    targetSeqFrontier    @4 :UInt64;
}

struct UnbanEntryPayload @0xbdd4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    obsoleteLamport              @1 :UInt64;
}

# ── Admission (join requests and decisions) ──────────────────────────

# How a community admits members. Set once by the creator; see
# GovernanceState::admission_mode.
enum AdmissionMode {
    open             @0;
    approvalRequired @1;
}

struct JoinRequestedPayload {
    requester            @0 :PseudonymKey;
    displayName          @1 :Text;
    obsoleteLamport              @2 :UInt64;
}

# Membership ops (plan E3.2). `JoinRequested` (@34) is absorbed into
# `MemberJoined`; `AdmissionPolicy` (@37) is retired because
# `GenesisCert.policy` pins the mode. Both arms are never read.
struct MemberJoinedPayload {
    memberRecord         @0 :Text;
    memberRecordOwner    @1 :Text;
    hasInviteProof       @2 :Bool;
    inviteCodeHash       @3 :Text;
    # The invite's join key's proof (64 bytes; `INVITE_PROOF`, plan E3.2
    # S10): Ed25519 under `inviteCodeHash`, the public half of the key
    # seeded from the invite code, over anchor, joiner, record and the hash.
    inviteSig            @4 :Data;
    # K_M's binding proof (64 bytes; `MEMBER_RECORD_BINDING`).
    ownerProof           @5 :Data;
    # The joiner's first device key, endorsed by its pseudonym (ADR 0022
    # rules 1 and 10, plan E3.2 S7d-keys.1); unset until a device key is
    # minted at join (S7d-keys.6).
    firstDevice          @6 :DeviceKeyEntry;
}

struct MemberRecordMovedPayload {
    memberRecord         @0 :Text;
    memberRecordOwner    @1 :Text;
    ownerProof           @2 :Data;
}

struct MemberLeftPayload {
    # The leaver's own channel slots as it held them when it left: a
    # backfilled post of the leaver is kept only from exactly the recorded
    # slot value, seq and digest (A/B finding R3-8, plan S9). Required: an op without
    # the list is refused, so every leave states its heads.
    heads                @0 :List(SlotHeadPayload);
}

struct MemberRemovedPayload {
    target               @0 :PseudonymKey;
    hasReason            @1 :Bool;
    reason               @2 :Text;
    # The removed member's channel slots as the remover held them: a
    # backfilled post of the member is kept only from exactly the recorded
    # slot value, seq and digest (A/B finding R3-8, plan S9).
    heads                @3 :List(SlotHeadPayload);
    # The number of the target's ops in the removal's causal past (s_k + 1;
    # 0: none): every later op of the target is masked (plan E3.2 S7c.3a).
    targetSeqFrontier    @4 :UInt64;
}

# A governance checkpoint (plan E3.2 S7c.3): an acceleration hint over a
# causal cut, never authoritative; the op is its author's endorsement. The
# digest, BLAKE3 derive_key(rekindle-gov-checkpoint-v1), covers every field
# but `pages` and `round`.
struct CheckpointEntry {
    height               @0 :UInt64;
    round                @1 :UInt32;
    hasPrevCheckpoint    @2 :Bool;          # false only at height 1
    prevCheckpoint       @3 :Op.OpRef;
    cut                  @4 :List(Op.OpRef); # the cut's heads, at most 63
    frontierHash         @5 :Data;          # 32 bytes
    dirRoot              @6 :Data;          # 32 bytes
    dirSize              @7 :UInt64;
    stateRoot            @8 :Data;          # 32 bytes
    mekGeneration        @9 :UInt64;
    eligibleSetHash      @10 :Data;         # 32 bytes
    pages                @11 :List(Text);   # directory page record keys
    stateSize            @12 :UInt32;       # bytes of the stateRoot preimage (S7c.4a)
}

# One element id of a member's device endorsement set (ADR 0022 rule 10):
# `(kind, deviceKey, incarnation)`.
enum DeviceSetElementKind {
    endorsement @0;
    retirement  @1;
}

struct DeviceSetElement {
    kind                 @0 :DeviceSetElementKind;
    deviceKey            @1 :Data;            # 32 bytes
    incarnation          @2 :UInt64;
}

# A pseudonym's endorsement of one of its per-community device keys (ADR
# 0022 rules 1, 2 and 10, plan E3.2 S7d-keys.1). `endorsementSig` is the
# pseudonym's Ed25519 over `rekindle-gov-device-key-v1 ‖ community ‖
# memberRecord ‖ deviceKey ‖ incarnation ‖ extendsSet ‖ deviceRecord ‖
# deviceRecordOwner ‖ forkEvidence`; `extendsSet` is BLAKE3
# derive_key(rekindle-gov-device-set-v1) over `extendsElements`, sorted.
struct DeviceKeyEntry {
    deviceKey            @0 :Data;            # 32 bytes
    incarnation          @1 :UInt64;
    extendsSet           @2 :Data;            # 32 bytes
    extendsElements      @3 :List(DeviceSetElement);
    deviceRecord         @4 :Text;            # DHT key of M(p,d)
    deviceRecordOwner    @5 :Text;            # K_M(p,d)'s public key text
    forkEvidence         @6 :Data;            # 32 bytes; zero when it answers none
    endorsementSig       @7 :Data;            # 64 bytes
}

# A pseudonym's retirement of one device key above `lastSeq` (ADR 0022
# rules 4, 5 and 10): Ed25519 by the pseudonym over
# `rekindle-gov-device-retired-v1 ‖ community ‖ memberRecord ‖ deviceKey ‖
# incarnation ‖ lastSeq`.
struct DeviceRetiredEntry {
    deviceKey            @0 :Data;            # 32 bytes
    incarnation          @1 :UInt64;
    lastSeq              @2 :UInt64;
    retirementSig        @3 :Data;            # 64 bytes
}

# A co-signed re-admission of a member's new device key after a fork (ADR
# 0022 rules 7, 8 and 10); its body under `rekindle-gov-device-readmit-v1`.
struct DeviceReadmitEntry {
    member               @0 :Data;            # the member's pseudonym, 32 bytes
    memberRecord         @1 :Text;            # M(p) from its MemberJoined
    newKey               @2 :Data;            # 32 bytes
    incarnation          @3 :UInt64;
    forkEvidence         @4 :Data;            # 32 bytes
}

# A successor community, a transfer offer, its withdrawal or its acceptance
# (ADR 0011, plan E3.2 S7s.3e, lead decisions (24)-(26)). A successor sets
# `newAnchor` (reason `recreate`); an offer sets `transferTo` and
# `creatorSetHash`; a withdrawal sets neither; an acceptance sets
# `newAnchor`, `creatorSetHash`, `accepts` (among the op's deps) and one
# link per new creator. A successor also sets `invitesRecord` (lead decision
# (34)): the first sealed DFLT(1) page of `SuccessorInvitesPage`s holding
# each continuing member's invite to the new community, HPKE-sealed to its
# card key. ADR 0022 rule 8's `cut`, `forkEvidence` and `pseudonymSig`
# append at @7 onward (S7d-keys K8).
struct CommunitySuccessorEntry {
    newAnchor            @0 :Text;            # empty in an offer or withdrawal
    reason               @1 :SuccessorReason;
    transferTo           @2 :Data;            # 32 bytes in an offer, else empty
    creatorSetHash       @3 :Data;            # 32 bytes in an offer or acceptance
    accepts              @4 :Op.OpRef;        # set in an acceptance only
    links                @5 :List(CreatorLink);
    invitesRecord        @6 :Text;            # set in a successor only
}

# One sealed page of a successor's invites (lead decision (34)), DFLT(1)
# written once, its owner key discarded (ADR 0011 "Sealed pages"). The first
# page, at the op's `invitesRecord`, lists the keys of the others in `pages`.
# Each entry is `HPKE Base` (RFC 9180, X25519/HKDF-SHA256/ChaCha20-Poly1305)
# sealed to `member`'s card `hpkeKey` under info
# `rekindle-successor-invite-v1` and AAD `old anchor || 0x00 || new anchor
# || member`; the plaintext is the invite link's text.
struct SuccessorInvitesPage {
    pages                @0 :List(Text);      # first page only
    entries              @1 :List(SealedInvite);
}

struct SealedInvite {
    member               @0 :Data;            # 32 bytes, old-community pseudonym
    enc                  @1 :Data;            # 32 bytes, HPKE encapsulated key
    ciphertext           @2 :Data;
}

enum SuccessorReason {
    recreate             @0;
    transfer             @1;
}

# A creator's old pseudonym bound to its new one, signed by `old` under
# `rekindle-gov-successor-link-v1` over old anchor || new anchor || `new`.
struct CreatorLink {
    old                  @0 :Data;            # 32 bytes
    new                  @1 :Data;            # 32 bytes
    sig                  @2 :Data;            # 64 bytes
}

# A member's request for a checkpoint's state snapshot (plan E3.2 S7c.3),
# carried in ControlPayload `checkpointStateRequest @77`: the bytes at
# `offset` of `canonical_state ‖ canonical_cut` at the cut of the checkpoint
# with this `height` and `digest`. Signed by `requester`, the asking
# member's pseudonym, over `rekindle-checkpoint-state-request-v1 ‖
# community ‖ height ‖ digest ‖ offset ‖ requester ‖ invite`, so the
# answering member serves admitted members, and joiners holding a live
# invite, only.
struct CheckpointStateRequest {
    community            @0 :Text;
    height               @1 :UInt64;
    digest               @2 :Data;          # 32 bytes, the checkpoint digest
    offset               @3 :UInt32;
    requester            @4 :Data;          # 32 bytes
    sig                  @5 :Data;          # 64 bytes
    # Empty: a member asks. Else the code hash of the live invite a joiner
    # holds (plan E3.2 S7c.5, the pre-join hint): the authority that lets it
    # join, under `sig` (the signed bytes add `0x01 ‖ len ‖ hash`, `0x00`
    # when empty).
    inviteCodeHash       @6 :Text;
    # With `inviteCodeHash`: `rekindle_secrets::invite::pre_join_read_tag`
    # under the invite code over the signed bytes (plan E3.2 S7c.8 (2)), so
    # only a holder of the code, never in the op log, is served. Empty for
    # a member.
    inviteTag            @7 :Data;
}

# A request for a usable checkpoint hint (plan E3.2 S7c.5), in ControlPayload
# `checkpointHintRequest @79`: with `height` 0 the hint selected at the
# greatest height, else the usable hint at `height` carrying `digest` (a
# joiner checking the endorsement chain down to the GenesisCert). Signed by
# `requester` over `rekindle-checkpoint-hint-request-v1 ‖ community ‖ height
# ‖ digest ‖ requester ‖ invite`; `inviteCodeHash` as in
# CheckpointStateRequest (empty: a member asks).
struct CheckpointHintRequest {
    community            @0 :Text;
    height               @1 :UInt64;
    digest               @2 :Data;          # 32 bytes; zeros with height 0
    requester            @3 :Data;          # 32 bytes
    inviteCodeHash       @4 :Text;
    sig                  @5 :Data;          # 64 bytes
    # As `CheckpointStateRequest.inviteTag` (plan E3.2 S7c.8 (2)).
    inviteTag            @6 :Data;
}

# The answer, in `checkpointHintResponse @80`: the hint's lowest
# `(lamport, op_hash)` `Checkpoint` op (unset: no usable hint, or the
# community holds a checkpoint equivocation) and the endorsements a strict
# majority of its electorate needs, lowest first, each located by author
# record and seq. Unsigned: the reader verifies every op it fetches.
struct CheckpointHintResponse {
    hasCheckpoint        @0 :Bool;
    checkpointRef        @1 :Op.OpRef;
    endorsements         @2 :List(Op.OpRef);
}

# The answer, in `checkpointStateResponse @78`: at most 30,000 of the
# `total` snapshot bytes from the request's offset. Unsigned: the reader
# checks the reassembled bytes against the checkpoint's `stateRoot`.
struct CheckpointStateResponse {
    total                @0 :UInt32;
    bytes                @1 :Data;
}

struct SlotHeadPayload {
    recordKey            @0 :Text;
    seq                  @1 :UInt32;
    # BLAKE3 derive_key digest of the slot value at `seq`: record key,
    # subkey, seq and the decrypted page (plan S9). Required, 32 bytes: a
    # departed member's post is kept only from exactly this value.
    digest               @2 :Data;
}

struct MemberApprovedPayload {
    target               @0 :PseudonymKey;
    obsoleteLamport              @1 :UInt64;
}

struct MemberRejectedPayload {
    target               @0 :PseudonymKey;
    hasReason            @1 :Bool;
    reason               @2 :Text;
    obsoleteLamport              @3 :UInt64;
}

struct AdmissionPolicyPayload {
    mode                 @0 :AdmissionMode;
    obsoleteLamport              @1 :UInt64;
}

struct TimeoutEntryPayload @0xbed4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    durationSeconds      @1 :UInt64;
    hasReason            @2 :Bool;
    reason               @3 :Text;
    startedAt            @4 :UInt64;
    obsoleteLamport              @5 :UInt64;
}

struct RemoveTimeoutEntryPayload @0xbfd4eb16e9bf1c2e {
    target               @0 :PseudonymKey;
    obsoleteLamport              @1 :UInt64;
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
    obsoleteLamport              @8 :UInt64;
}

struct CommunityNotificationDefaultEntry @0xc1d4eb16e9bf1c2e {
    # `all` / `mentions` / `nothing`.
    level                @0 :Text;
    obsoleteLamport              @1 :UInt64;
}

struct MEKGenerationBumpEntry @0xc2d4eb16e9bf1c2e {
    generation           @0 :UInt64;
    triggerDeparted      @1 :PseudonymKey;
    cascadeSkipped       @2 :List(PseudonymKey);
    obsoleteLamport              @3 :UInt64;
}

struct CategoryCreatedEntry @0xc3d4eb16e9bf1c2e {
    categoryId           @0 :Uuid16;
    name                 @1 :Text;
    position             @2 :UInt32;
    obsoleteLamport              @3 :UInt64;
}

struct CategoryArchivedEntry @0xc4d4eb16e9bf1c2e {
    categoryId           @0 :Uuid16;
    obsoleteLamport              @1 :UInt64;
}

struct PermissionOverwriteEntry @0xc5d4eb16e9bf1c2e {
    channelId            @0 :Uuid16;
    # `role` or `member`.
    targetType           @1 :Text;
    # Role ID hex or member pseudonym hex.
    targetId             @2 :Text;
    allow                @3 :UInt64;
    deny                 @4 :UInt64;
    obsoleteLamport              @5 :UInt64;
}

struct ThreadCreatedEntry @0xc6d4eb16e9bf1c2e {
    threadId             @0 :Uuid16;
    parentChannelId      @1 :Uuid16;
    name                 @2 :Text;
    threadType           @3 :Text;
    # Obsolete (plan E3.3, T1): a thread has no record.
    obsoleteHasRecordKey @4 :Bool;
    obsoleteRecordKey    @5 :Text;
    invited              @6 :List(PseudonymKey);
    hasForumTag          @7 :Bool;
    forumTag             @8 :Text;
    autoArchiveSeconds   @9 :UInt64;
    obsoleteLamport              @10 :UInt64;
}

struct ThreadArchivedEntry @0xc7d4eb16e9bf1c2e {
    threadId             @0 :Uuid16;
    obsoleteLamport              @1 :UInt64;
}

struct ThreadUnarchivedEntry @0xaf718751aeb31c5b {
    threadId             @0 :Uuid16;
    obsoleteLamport              @1 :UInt64;
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
    obsoleteLamport              @19 :UInt64;
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
    obsoleteLamport              @16 :UInt64;
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
    obsoleteLamport              @1 :UInt64;
}

struct EventArchivedEntry @0xccd4eb16e9bf1c2e {
    eventId              @0 :Uuid16;
    obsoleteLamport              @1 :UInt64;
}

# Plan C7.15 — the author's RSVP to a live event; LWW per (event, author).
struct EventRsvpEntry @0x822acc59ac9bfa9e {
    eventId              @0 :Uuid16;
    status               @1 :RsvpStatus;
    obsoleteLamport              @2 :UInt64;
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
    obsoleteLamport              @7 :UInt64;
}

struct WelcomeScreenEntry @0xced4eb16e9bf1c2e {
    description          @0 :Text;
    channels             @1 :List(WelcomeChannel);
    obsoleteLamport              @2 :UInt64;
}

struct AdminDeleteEntry @0xcfd4eb16e9bf1c2e {
    messageId            @0 :Uuid16;
    channelId            @1 :Uuid16;
    hasReason            @2 :Bool;
    reason               @3 :Text;
    obsoleteLamport              @4 :UInt64;
    # The removed post's author: M1 ranks the moderator against it, and
    # readers apply the delete only to a post by it (plan E3.3).
    author               @5 :PseudonymKey;
}

# Obsolete (plan E3.3): a channel has no record in any segment; posts
# ride each author's `M`/4 outbox. Kept for its id; no arm reads it.
struct ChannelSegmentLinkedEntry @0xd0d4eb16e9bf1c2e {
    channelId            @0 :Uuid16;
    segmentIndex         @1 :UInt32;
    recordKey            @2 :Text;
    obsoleteLamport              @3 :UInt64;
}

struct SegmentAddedEntry @0xd1d4eb16e9bf1c2e {
    segmentIndex         @0 :UInt32;
    registryKey          @1 :Text;
    anchor        @2 :Text;
    slotRangeStart       @3 :UInt32;
    slotRangeEnd         @4 :UInt32;
    obsoleteLamport              @5 :UInt64;
}

struct AutoModRuleEntry @0xd2d4eb16e9bf1c2f {
    ruleId               @0 :Uuid16;
    name                 @1 :Text;
    enabled              @2 :Bool;
    triggerJson          @3 :Text;
    # The free-text action this field carried; never written or read
    # (D26: no ordinal reuse). `actionKind` carries the action.
    obsoleteAction       @4 :Text;
    obsoleteLamport              @5 :UInt64;
    actionKind           @6 :AutoModActionKind;
}

enum AutoModActionKind {
    blockLocally         @0;
    blurContent          @1;
    alertModerators      @2;
}

struct RoleArchivedEntry @0xd3d4eb16e9bf1c2e {
    roleId               @0 :Uuid16;
    obsoleteLamport              @1 :UInt64;
}

struct CategoryUpdatedEntry @0xd4d4eb16e9bf1c2e {
    categoryId           @0 :Uuid16;
    hasName              @1 :Bool;
    name                 @2 :Text;
    hasPosition          @3 :Bool;
    position             @4 :UInt32;
    obsoleteLamport              @5 :UInt64;
}

struct InviteCreatedEntry @0xd5d4eb16e9bf1c2e {
    inviteId             @0 :Uuid16;
    codeHash             @1 :Text;
    maxUses              @2 :UInt32;
    hasExpiresAt         @3 :Bool;
    expiresAt            @4 :UInt64;
    secretsRecordKey     @5 :Text;
    obsoleteLamport              @6 :UInt64;
}

struct InviteRevokedEntry @0xd6d4eb16e9bf1c2e {
    inviteId             @0 :Uuid16;
    obsoleteLamport              @1 :UInt64;
}

struct AttachmentPinnedEntry @0xd7d4eb16e9bf1c2e {
    attachmentId         @0 :Uuid16;
    pinned               @1 :Bool;
    obsoleteLamport              @2 :UInt64;
}

struct CommunityPolicyEntry @0xd8d4eb16e9bf1c2e {
    hasPolicyText        @0 :Bool;
    policyText           @1 :Text;
    maxJoinsPerInterval  @2 :UInt32;
    joinIntervalSeconds  @3 :UInt32;
    obsoleteLamport              @4 :UInt64;
}

# A report to the moderators (plan E3.1, D26): a message or a member.
struct ModReportEntry @0xe019ff96c073fa38 {
    reportId             @0 :Uuid16;
    target :union {
        message :group {
            channelId    @1 :Uuid16;
            messageId    @2 :Text;
        }
        member           @3 :PseudonymKey;
    }
    reason               @4 :Text;
    obsoleteLamport              @5 :UInt64;
}

# Top-level governance entry union — 40 arms, append-only.
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
        obsoleteChannelSegmentLinked  @25 :ChannelSegmentLinkedEntry;
        segmentAdded                  @26 :SegmentAddedEntry;
        autoModRule                   @27 :AutoModRuleEntry;
        roleArchived                  @28 :RoleArchivedEntry;
        categoryUpdated               @29 :CategoryUpdatedEntry;
        inviteCreated                 @30 :InviteCreatedEntry;
        inviteRevoked                 @31 :InviteRevokedEntry;
        attachmentPinned              @32 :AttachmentPinnedEntry;
        communityPolicy               @33 :CommunityPolicyEntry;
        obsoleteJoinRequested         @34 :JoinRequestedPayload;
        memberApproved                @35 :MemberApprovedPayload;
        memberRejected                @36 :MemberRejectedPayload;
        obsoleteAdmissionPolicy       @37 :AdmissionPolicyPayload;
        eventRsvp                     @38 :EventRsvpEntry;
        modReport                     @39 :ModReportEntry;
        threadUnarchived              @40 :ThreadUnarchivedEntry;
        memberJoined                  @41 :MemberJoinedPayload;
        memberRecordMoved             @42 :MemberRecordMovedPayload;
        memberLeft                    @43 :MemberLeftPayload;
        memberRemoved                 @44 :MemberRemovedPayload;
        checkpoint                    @45 :CheckpointEntry;
        communitySuccessor            @46 :CommunitySuccessorEntry;
        deviceKey                     @47 :DeviceKeyEntry;
        deviceRetired                 @48 :DeviceRetiredEntry;
        deviceReadmit                 @49 :DeviceReadmitEntry;
    }
}
