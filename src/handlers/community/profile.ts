import { commands } from "../../ipc/commands";
import { setCommunityState, communityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";

export async function handleUpdateCommunityProfile(
  communityId: string,
  bio: string | null,
  pronouns: string | null,
  themeColor: number | null,
  badges: string[],
  avatarRef: string | null = null,
  bannerRef: string | null = null,
): Promise<boolean> {
  try {
    await commands.updateCommunityProfile(
      communityId,
      bio,
      pronouns,
      themeColor,
      badges,
      avatarRef,
      bannerRef,
    );
    setCommunityState("communities", communityId, {
      myBio: bio,
      myPronouns: pronouns,
      myThemeColor: themeColor,
      myBadges: badges,
      myAvatarRef: avatarRef,
      myBannerRef: bannerRef,
    });
    const community = communityState.communities[communityId];
    const myPseudonymKey = community?.myPseudonymKey;
    if (community && myPseudonymKey) {
      const memberIdx = community.members.findIndex((m) => m.pseudonymKey === myPseudonymKey);
      if (memberIdx >= 0) {
        setCommunityState("communities", communityId, "members", memberIdx, {
          bio,
          pronouns,
          themeColor,
          badges,
          avatarRef,
          bannerRef,
        });
      }
    }
    addToast("Profile updated", "success");
    return true;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Failed to update profile";
    console.error("Failed to update community profile:", e);
    addToast(msg, "error");
    return false;
  }
}

export async function handleUploadAttachment(
  communityId: string,
  channelId: string,
  filePath: string,
): Promise<string | null> {
  try {
    const id = await commands.uploadAttachment(communityId, channelId, filePath);
    addToast("File uploaded", "success");
    return id;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Upload failed";
    console.error("Upload failed:", e);
    addToast(msg, "error");
    return null;
  }
}

export async function handleDownloadAttachment(
  communityId: string,
  channelId: string,
  attachmentId: string,
  defaultFilename: string,
): Promise<boolean> {
  try {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const savePath = await save({ defaultPath: defaultFilename });
    if (!savePath) return false;
    await commands.downloadAttachment(communityId, channelId, attachmentId, savePath as string);
    return true;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Download failed";
    console.error("Download failed:", e);
    addToast(msg, "error");
    return false;
  }
}

export async function handlePinAttachment(
  communityId: string,
  attachmentId: string,
  pinned: boolean,
): Promise<void> {
  try {
    await commands.pinAttachment(communityId, attachmentId, pinned);
    addToast(pinned ? "Attachment pinned" : "Attachment unpinned", "success");
  } catch (e) {
    const msg = typeof e === "string" ? e : "Pin update failed";
    console.error("Pin update failed:", e);
    addToast(msg, "error");
  }
}

/**
 * Plate Gate (architecture §15): admin expands the community to a new
 * SMPL segment when the highest existing segment hits its 255-slot cap.
 * Backend validates `MANAGE_COMMUNITY` and that the segment is full;
 * returns the new `segment_index`.
 */
export async function handleExpandCommunitySegment(
  communityId: string,
): Promise<number | null> {
  try {
    const newIndex = await commands.expandCommunitySegment(communityId);
    addToast(`Community expanded — segment ${newIndex} ready for new members`, "success");
    return newIndex;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Expansion failed";
    console.error("Plate Gate expansion failed:", e);
    addToast(msg, "error");
    return null;
  }
}

export async function handleUpdateCommunityPresence(
  communityId: string,
  status: string,
): Promise<void> {
  try {
    await commands.updateCommunityPresence(communityId, status);
  } catch (e) {
    console.error("Failed to update community presence:", e);
  }
}

export async function handleSetNotificationOverride(
  communityId: string, channelId: string, level: "all" | "mentions" | "nothing"
): Promise<void> {
  try {
    await commands.setChannelNotificationLevel(communityId, channelId, level);
    const community = communityState.communities[communityId];
    if (!community) return;
    const index = community.channels.findIndex((channel) => channel.id === channelId);
    if (index >= 0) {
      setCommunityState("communities", communityId, "channels", index, "notificationLevel", level);
    }
  } catch (e) {
    console.error("Failed to update channel notification level:", e);
    addToast("Failed to update notification settings", "error");
  }
}

// Architecture §32 Phase 7 Week 25 — channel-level notification sound
// override. `soundRef` is the soundboard expression's content_hash;
// passing `null` clears the override and re-inherits from the
// community default → app default cascade (resolved server-side in
// `services/community/notifications.rs::resolve_notification_sound`).
export async function handleSetChannelNotificationSound(
  communityId: string,
  channelId: string,
  soundRef: string | null,
): Promise<void> {
  try {
    await commands.setNotificationSound(communityId, channelId, soundRef);
    const community = communityState.communities[communityId];
    if (!community) return;
    const index = community.channels.findIndex((channel) => channel.id === channelId);
    if (index >= 0) {
      setCommunityState(
        "communities",
        communityId,
        "channels",
        index,
        "notificationSoundRef",
        soundRef,
      );
    }
    addToast("Notification sound updated", "success");
  } catch (e) {
    console.error("Failed to update channel notification sound:", e);
    addToast("Failed to update sound", "error");
  }
}

// ── Onboarding & Welcome Screen ──

export async function handleLoadOnboardingConfig(communityId: string): Promise<void> {
  try {
    const config = await commands.getOnboardingConfig(communityId);
    setCommunityState("communities", communityId, "onboardingConfig", config);
  } catch (e) {
    console.error("Failed to load onboarding config:", e);
  }
}

export async function handleLoadWelcomeScreen(communityId: string): Promise<void> {
  try {
    const screen = await commands.getWelcomeScreen(communityId);
    setCommunityState("communities", communityId, "welcomeScreen", screen);
  } catch (e) {
    console.error("Failed to load welcome screen:", e);
  }
}

export async function handleSubmitOnboarding(
  communityId: string,
  answers: { questionId: string; selectedOptions: string[] }[],
  acknowledgedRules?: boolean,
): Promise<boolean> {
  try {
    await commands.submitOnboardingAnswers(communityId, answers, acknowledgedRules);
    // Plan §Failure 8 — persist the completion flag in SQLite so the
    // wizard doesn't re-trigger on the next launch. The mesh broadcast
    // started by `submit_onboarding_answers` covers other peers; this
    // covers the local device (send_to_mesh excludes loopback).
    await commands.markOnboardingComplete(communityId);
    setCommunityState("communities", communityId, "onboardingComplete", true);
    return true;
  } catch (e) {
    console.error("Failed to submit onboarding:", e);
    addToast(`Failed to complete onboarding: ${String(e)}`, "error");
    return false;
  }
}
