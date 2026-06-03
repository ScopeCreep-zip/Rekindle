import { commands } from "../../ipc/commands";
import { setCommunityState, communityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";
import {
  transformAutoModRule,
  transformCommunityDetail,
  transformExpression,
  transformMember,
} from "../../utils/transformers";
import { handleLoadUnreadCounts } from "./channels";
import { handleUpdateCommunityPresence } from "./profile";

export async function handleCreateCommunity(name: string): Promise<void> {
  try {
    const id = await commands.createCommunity(name);
    // Fetch full community details from backend (includes pseudonym, MEK gen, channels, roles)
    const details = await commands.getCommunityDetails();
    const created = details.find((c) => c.id === id);
    if (created) {
      setCommunityState("communities", id, transformCommunityDetail(created));
    } else {
      setCommunityState("communities", id, {
        id,
        name,
        description: null,
        channels: [],
        categories: [],
        members: [],
        roles: [],
        myRoleIds: [0, 1],
        myPseudonymKey: null,
        mekGeneration: 0,
        events: [],
        expressions: [],
        automodRules: [],
      });
    }
    // Fetch members so the creator appears in the member list
    try {
      const members = await commands.getCommunityMembers(id);
      setCommunityState("communities", id, "members", members.map(transformMember));
    } catch (e) {
      console.error("Failed to load community members after creation:", e);
    }
  } catch (e) {
    console.error("Failed to create community:", e);
    const msg = typeof e === "string" ? e : "Failed to create community";
    addToast(msg, "error");
  }
}

export async function handleJoinCommunity(
  communityId: string,
  name: string,
  inviteCode?: string,
): Promise<void> {
  try {
    await commands.joinCommunity(communityId, inviteCode);
    // Re-fetch community details to get channels, pseudonym key, MEK generation, roles
    const details = await commands.getCommunityDetails();
    const joined = details.find((c) => c.id === communityId);
    if (joined) {
      setCommunityState("communities", communityId, transformCommunityDetail(joined));
    } else {
      setCommunityState("communities", communityId, {
        id: communityId,
        name,
        description: null,
        channels: [],
        categories: [],
        members: [],
        roles: [],
        myRoleIds: [0, 1],
        myPseudonymKey: null,
        mekGeneration: 0,
        events: [],
        expressions: [],
        automodRules: [],
      });
    }
    // Fetch members for the newly joined community
    try {
      const members = await commands.getCommunityMembers(communityId);
      setCommunityState("communities", communityId, "members", members.map(transformMember));
    } catch (e) {
      console.error("Failed to load community members after join:", e);
    }
    await handleLoadExpressions(communityId);
    await handleLoadAutoModRules(communityId);

    // Auto-select the newly joined community so it appears immediately
    handleSelectCommunity(communityId);
    addToast("Joined community!", "success");
  } catch (e) {
    // Surface the backend's specific error string (banned / full / invalid invite / Stronghold locked /
    // a timed-out join phase / etc.) rather than swallowing it as a generic "Failed to join community" —
    // the user can't act on a message that hides the cause. Re-throw so the join modal stays open and
    // shows the failure inline next to the dial-in stepper's failed phase (programmatic callers such as
    // the deep-link handler catch this themselves).
    console.error("Failed to join community:", e);
    const msg = typeof e === "string" ? e : "Failed to join community";
    addToast(msg, "error");
    throw e instanceof Error ? e : new Error(msg);
  }
}

export function handleSelectCommunity(communityId: string): void {
  setCommunityState("activeCommunity", communityId);
  // Fetch members for the selected community
  commands.getCommunityMembers(communityId).then((members) => {
    setCommunityState("communities", communityId, "members", members.map(transformMember));
  }).catch((e) => {
    console.error("Failed to load community members:", e);
    addToast("Failed to load members", "error");
  });
  // Notify the server we're online in this community
  handleUpdateCommunityPresence(communityId, "online");
  // Refresh community details to ensure myPseudonymKey, roles, categories, and mekGeneration are current
  commands.getCommunityDetails().then((details) => {
    const detail = details.find((c) => c.id === communityId);
    if (detail) {
      setCommunityState("communities", communityId, "myPseudonymKey", detail.myPseudonymKey ?? null);
      setCommunityState("communities", communityId, "mekGeneration", detail.mekGeneration ?? 0);
      setCommunityState("communities", communityId, "myRoleIds", detail.myRoleIds ?? [0, 1]);
      setCommunityState("communities", communityId, "roles", detail.roles ?? []);
      setCommunityState("communities", communityId, "description", detail.description ?? null);
      setCommunityState("communities", communityId, "categories", detail.categories ?? []);
    }
  }).catch((e) => {
    console.error("Failed to refresh community details:", e);
    addToast("Failed to refresh community", "error");
  });
  void handleLoadExpressions(communityId);
  // Fetch unread counts for all channels in this community
  handleLoadUnreadCounts(communityId);
}

export async function handleLoadExpressions(communityId: string): Promise<void> {
  try {
    const expressions = await commands.listExpressions(communityId);
    setCommunityState("communities", communityId, "expressions", expressions.map(transformExpression));
  } catch (e) {
    console.error("Failed to load expressions:", e);
  }
}

export async function handleLoadAutoModRules(communityId: string): Promise<void> {
  try {
    const rules = await commands.listAutoModRules(communityId);
    setCommunityState("communities", communityId, "automodRules", rules.map(transformAutoModRule));
  } catch (e) {
    console.error("Failed to load automod rules:", e);
  }
}

// Architecture §32 Phase 5 Week 15 — resolve `iconHash` and
// `bannerHash` to `data:image/webp;base64,...` URLs and cache them on
// the community store. The icon is rendered in
// `CommunityListCompact.tsx` and `CommunityWindow` headers; resolving
// once on hydration avoids re-fetching the base64 on every render.
// The setter (`set_community_avatar`) returns a hash that the caller
// can pass to `updateCommunityInfo` — when the new hash arrives via
// the `communityUpdated` event we'll re-resolve on demand.
export async function handleResolveCommunityImageDataUrls(
  communityId: string,
): Promise<void> {
  const community = communityState.communities[communityId];
  if (!community) return;

  const iconHash = community.iconHash ?? null;
  const bannerHash = community.bannerHash ?? null;

  if (iconHash) {
    try {
      const url = await commands.getCommunityAvatarDataUrl(communityId, iconHash);
      setCommunityState("communities", communityId, "iconDataUrl", url ?? null);
    } catch (e) {
      console.error(`Failed to resolve community icon for ${communityId}:`, e);
      setCommunityState("communities", communityId, "iconDataUrl", null);
    }
  } else {
    setCommunityState("communities", communityId, "iconDataUrl", null);
  }

  if (bannerHash) {
    try {
      const url = await commands.getCommunityAvatarDataUrl(communityId, bannerHash);
      setCommunityState("communities", communityId, "bannerDataUrl", url ?? null);
    } catch (e) {
      console.error(`Failed to resolve community banner for ${communityId}:`, e);
      setCommunityState("communities", communityId, "bannerDataUrl", null);
    }
  } else {
    setCommunityState("communities", communityId, "bannerDataUrl", null);
  }
}

export async function handleSetAutoModRule(
  communityId: string,
  rule: {
    ruleId?: string | null;
    name: string;
    enabled: boolean;
    keywords: string[];
    regexPatterns: string[];
    action: "block_locally" | "blur_content" | "alert_moderators";
  },
): Promise<void> {
  try {
    await commands.setAutoModRule(
      communityId,
      rule.ruleId ?? null,
      rule.name,
      rule.enabled,
      rule.keywords,
      rule.regexPatterns,
      rule.action,
    );
    await handleLoadAutoModRules(communityId);
  } catch (e) {
    console.error("Failed to save automod rule:", e);
    addToast("Failed to save AutoMod rule", "error");
    throw e;
  }
}

export async function handleDeleteAutoModRule(
  communityId: string,
  ruleId: string,
): Promise<void> {
  try {
    await commands.deleteAutoModRule(communityId, ruleId);
    await handleLoadAutoModRules(communityId);
  } catch (e) {
    console.error("Failed to delete automod rule:", e);
    addToast("Failed to delete AutoMod rule", "error");
    throw e;
  }
}

export async function handleUploadEmoji(
  communityId: string,
  name: string,
  bytes: number[],
  animated: boolean,
): Promise<string | null> {
  try {
    const expressionId = await commands.uploadEmoji(communityId, name, bytes, animated);
    await handleLoadExpressions(communityId);
    addToast("Emoji uploaded", "success");
    return expressionId;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Failed to upload emoji";
    console.error("Failed to upload emoji:", e);
    addToast(msg, "error");
    return null;
  }
}

// Architecture §18.2 — sticker upload (Lost Cargo, eager-cached).
export async function handleUploadSticker(
  communityId: string,
  name: string,
  bytes: number[],
  animated: boolean,
  tags?: string[],
): Promise<string | null> {
  try {
    const expressionId = await commands.uploadSticker(communityId, name, bytes, animated, tags);
    await handleLoadExpressions(communityId);
    addToast("Sticker uploaded", "success");
    return expressionId;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Failed to upload sticker";
    console.error("Failed to upload sticker:", e);
    addToast(msg, "error");
    return null;
  }
}

// Architecture §18.3 — soundboard sound upload. Caller supplies
// duration measured by `decodeAudioData`, volume in 0.0..=1.0 and
// optional emoji glyph; backend rejects clips longer than 5 s.
export async function handleUploadSoundboardSound(
  communityId: string,
  name: string,
  bytes: number[],
  durationSeconds: number,
  volume: number,
  emoji?: string,
  tags?: string[],
): Promise<string | null> {
  try {
    const expressionId = await commands.uploadSoundboardSound(
      communityId,
      name,
      bytes,
      durationSeconds,
      volume,
      emoji,
      tags,
    );
    await handleLoadExpressions(communityId);
    addToast("Soundboard sound uploaded", "success");
    return expressionId;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Failed to upload sound";
    console.error("Failed to upload soundboard sound:", e);
    addToast(msg, "error");
    return null;
  }
}

// Architecture §10.9 — trigger soundboard playback in the active
// voice channel. Receivers fetch the cached audio and play locally.
export async function handlePlaySoundboard(
  communityId: string,
  channelId: string,
  expressionId: string,
): Promise<void> {
  try {
    await commands.playSoundboard(communityId, channelId, expressionId);
  } catch (e) {
    console.error("Failed to play soundboard sound:", e);
    addToast("Failed to play sound", "error");
  }
}

export async function handleLeaveCommunity(communityId: string): Promise<void> {
  try {
    await commands.leaveCommunity(communityId);
    setCommunityState("communities", (prev) => {
      const next = { ...prev };
      delete next[communityId];
      return next;
    });
    // If we were viewing this community, clear the selection
    if (communityState.activeCommunity === communityId) {
      setCommunityState("activeCommunity", null);
      setCommunityState("activeChannel", null);
    }
  } catch (e) {
    console.error("Failed to leave community:", e);
    addToast("Failed to leave community", "error");
  }
}
