import type {
  CatchUpView,
  KeyStatus,
  OpPublicationState,
} from "../channels/community_governance_events";
import { hostInvoke, invoke } from "../invoke";
import type { ImageKind } from "./system";

/// A voice message as the Host decoded it: the Ogg Opus audio and the
/// duration and waveform read from it (base64 fields).
/** This device's standing in a community (`DeviceStatus`, plan E3.2 / ADR 0022 rule 4):
 *  `needsReenrol` while it holds no key endorsed for our current member record. */
export type DeviceStatus = "enrolled" | "needsReenrol";

/** A governance write's answer (`OpOutcome`, plan E3.2 S7d-keys.8): `pending` while the
 *  outbox has not landed the op; `heldOpId` names a write the unlock barrier holds, unsigned,
 *  as the community's `CatchUp` pending list lists it (null once signed). */
export interface GovernanceWriteAnswer {
  pending: boolean;
  heldOpId: string | null;
}

export interface VoiceMessageAudioView {
  audio: string;
  durationMs: number;
  waveform: string;
}

import type { CommunityProfilePatch, MemberGroup, ProfileField } from "./dto";
import type {
  ChannelDeniedCapabilities,
  DeniedCapabilities,
  MemberModeration,
} from "./dto_capabilities";

/** `SuccessorOffer` (rekindle-types `dto::community`): a successor both
 *  sides link, as `communitySuccessorOffered` carries it. */
export interface SuccessorOffer {
  community: string;
  newAnchor: string;
  signer: string;
  reason: "recreate" | "transfer";
  creators: string[];
}

/** `SuccessorLinkRequested` (rekindle-types `dto::community`): a
 *  transfer's link request asking us, as `successorLinkRequested` carries it. */
interface SuccessorLinkRequested {
  community: string;
  request: string;
  recipient: string;
  newAnchor: string;
}

/** `CommunitySuccessors` (rekindle-types `dto::community`): the successor
 *  snapshot, offers this session and link requests asking us now. */
export interface CommunitySuccessors {
  offers: SuccessorOffer[];
  linkRequests: SuccessorLinkRequested[];
}

/** `CommunityCreated` (rekindle-types `dto::community`). */
export interface CommunityCreated {
  anchor: string;
  registryKey: string;
  name: string;
  pendingCoCreators: string[];
}

/// The held co-creator invites and our pending creations, as the Host
/// holds them (`CoCreationSnapshot`, plan E3.2 S7s.3g).
export interface CoCreationSnapshot {
  invites: { anchor: string; peerKey: string; communityName: string }[];
  creations: {
    anchor: string;
    name: string;
    coCreators: { peerKey: string; status: string }[];
  }[];
}

/** The community's own lifecycle: views, profile, creation and co-creation,
 *  successors and joining. Channel content and administration are in
 *  community_content.ts and community_admin.ts. */
export const communityCommands = {
  // Community
  getCommunities: () =>
    hostInvoke<
      {
        anchor: string;
        name: string;
        description: string;
        memberCount: number;
        channelCount: number;
        ourPseudonym: string;
      }[]
    >("get_communities"),
  getCommunityDetails: () =>
    hostInvoke<
      {
        id: string;
        name: string;
        description: string | null;
        iconHash?: string | null;
        bannerHash?: string | null;
        channels: {
          id: string;
          name: string;
          channelType: string;
          categoryId?: string;
          topic?: string;
          forumTags?: string[];
          stageSpeakers?: string[];
          stageModerator?: string | null;
          notificationLevel?: "all" | "mentions" | "nothing";
          notificationSoundRef?: string | null;
          locked: boolean;
          lockedReason: string;
          denied: ChannelDeniedCapabilities;
        }[];
        categories?: { id: string; name: string; sortOrder: number }[];
        myRole: string | null;
        myRoleIds: string[];
        myPermissions: string;
        myChannelPermissions: Record<string, string>;
        roles: {
          id: string;
          name: string;
          color: number;
          tagColours: { dark: number; light: number };
          permissions: string;
          position: number;
          /** The position as the Host presents it (contract PN1: the owner role reads
           *  "Always first", never the raw sentinel). */
          positionLabel: string;
          hoist: boolean;
          mentionable: boolean;
          selfAssignable?: boolean;
          exclusionGroup?: string;
        }[];
        myPseudonymKey: string | null;
        mekGeneration: number;
        onboardingComplete: boolean;
        /** We hold a creator seat now (`CommunityView::we_are_creator`); re-read on `effectivePermissionsChanged` naming us. */
        weAreCreator: boolean;
        /** The controls we may not use, each with the Host's reason (`CommunityView::denied`); replaced from `effectivePermissionsChanged` naming us. */
        denied: DeniedCapabilities;
        /** Our name here, never empty: our per-community name, else our short fingerprint (plan E3.4 S6). */
        myDisplayName: string;
        /** Our per-community name as set; absent when unset (the set-name form's prefill). */
        myCardName?: string;
        /** Our card's custom status; absent when none. */
        myCustomStatus?: string;
        myBio?: string | null;
        myPronouns?: string | null;
        /** Our saved picture refs (PF2); absent when none is saved. */
        myAvatarRef?: string;
        myBannerRef?: string;
        myThemeColor?: number | null;
        myBadges?: string[];
        removed?: {
          banned: boolean;
          by: string | null;
          byName: string | null;
          reason?: string | null;
        } | null;
        /** Our ops not yet landed, by op hash (`CommunityView::op_publications`). */
        opPublications?: Record<string, OpPublicationState>;
        /** Whether this device may write here (`CommunityView::device_status`, ADR 0022 rule 4). */
        deviceStatus: DeviceStatus;
        /** The unlock barrier's held governance writes (`CommunityView::catch_up`); later changes arrive as `catchUp`. */
        catchUp?: CatchUpView;
        /** Our community key, as the Host states it (`CommunityView::key_status`); changes arrive as `keyStatus`. */
        keyStatus: KeyStatus;
      }[]
    >("get_community_details"),
  getCommunityMembers: (communityId: string) =>
    hostInvoke<
      {
        pseudonymKey: string;
        /** Never empty: the member's per-community name, else their short fingerprint (plan E3.4 S6). */
        displayName: string;
        /** Always the short pseudonym fingerprint ("a1b2 c3d4"), for a secondary line or tooltip. */
        fingerprint: string;
        /** Their card's custom status; absent when none. */
        customStatus?: string;
        roleIds: string[];
        displayRole: string;
        status: string;
        timeoutUntil: number | null;
        bio?: string | null;
        pronouns?: string | null;
        themeColor?: number | null;
        badges?: string[];
        avatarRef?: string | null;
        bannerRef?: string | null;
        location?: { kind: "text" | "voice"; channelId: string } | null;
        lastActive: number;
        group: MemberGroup;
        moderation: MemberModeration;
        outranks: boolean;
      }[]
    >("get_community_members", { communityId }),
  /**
   * Architecture §32 Phase 5 W15 — upload + cache the local user's
   * per-community avatar. Compresses to 128×128 WebP on the backend
   * and returns the BLAKE3 hex hash of the compressed bytes; pass the
   * hash to `updateCommunityProfile` as `avatarRef`.
   */
  setCommunityAvatar: (communityId: string, bytes: number[], kind: ImageKind) =>
    hostInvoke<string>("set_community_avatar", { communityId, bytes, kind }),
  /** Same as above for banner (compressed to 600×200). */
  setCommunityBanner: (communityId: string, bytes: number[], kind: ImageKind) =>
    hostInvoke<string>("set_community_banner", { communityId, bytes, kind }),
  /**
   * Resolve a stored avatar/banner blob to a `data:image/webp;base64,…`
   * URL ready to drop into an `<img src=…>`. Returns `null` if the
   * hash isn't cached locally yet (peer fetch rides Lost Cargo and
   * isn't part of this command).
   */
  getCommunityAvatarDataUrl: (communityId: string, hash: string) =>
    hostInvoke<string | null>("get_community_avatar_data_url", { communityId, hash }),
  /** Our card's edit in this community (plan PF2): each field unchanged, cleared or set. */
  updateCommunityProfile: (communityId: string, patch: CommunityProfilePatch) =>
    hostInvoke<void>("update_community_profile", { communityId, patch }),
  /** Our name in this community (`CommunitySetDisplayName`, the Host's review
   *  SetCommunityDisplayName); a clear shows the fingerprint (PF2). */
  setCommunityDisplayName: (communityId: string, displayName: ProfileField<string>) =>
    hostInvoke<void>("set_community_display_name", { communityId, displayName }),
  createCommunity: (name: string) =>
    hostInvoke<CommunityCreated>("create_community", { name }).then((created) => created.anchor),
  /// Creates with friends named co-creators (plan E3.2 S7s.3): the
  /// genesis waits for each to accept, decline or be removed, so the
  /// result lists the co-creators still pending.
  createCommunityWithCoCreators: (name: string, coCreators: string[]) =>
    hostInvoke<CommunityCreated>("create_community", { name, coCreators }),
  /// Read once the window's event stream is live (plan VI1's pattern).
  getCoCreation: () => invoke<CoCreationSnapshot>("get_co_creation"),
  /// Re-creates the community under a new anchor naming it as
  /// predecessor, with us as sole creator, and offers it (plan E3.2 S7s.4);
  /// `continuing` (pseudonym hex) get a sealed invite, absent every member.
  publishCommunitySuccessor: (communityId: string, continuing?: string[]) =>
    hostInvoke<CommunityCreated>("publish_community_successor", { communityId, continuing }),
  /// Follows an offered successor with the invite sealed to us in its
  /// invites record; the old community is kept.
  followCommunitySuccessor: (communityId: string, newAnchor: string) =>
    hostInvoke<{ anchor: string; name: string }>("follow_community_successor", {
      communityId,
      newAnchor,
    }),
  /// The successor snapshot (lead decisions (33), (46)): the offers this
  /// session and the link requests asking us, read after the event stream
  /// is live.
  getCommunitySuccessors: (communityId: string) =>
    hostInvoke<CommunitySuccessors>("get_community_successors", { communityId }),
  removeCoCreator: (communityId: string, identity: string) =>
    hostInvoke<void>("remove_co_creator", { communityId, identity }),
  cancelCommunityCreate: (communityId: string) =>
    hostInvoke<void>("cancel_community_create", { communityId }),
  answerCreatorInvite: (communityId: string, accept: boolean) =>
    hostInvoke<void>("answer_creator_invite", { communityId, accept }),
  /// Joins through the Host's review of the invite (its record-key
  /// fingerprint); returns the joined community's id.
  joinCommunity: (inviteUrl: string) =>
    hostInvoke<{ anchor: string; name: string }>("join_community", { inviteUrl }).then(
      (joined) => joined.anchor
    ),
};
