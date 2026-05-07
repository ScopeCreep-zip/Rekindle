import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { subscribeNotificationEvents } from "../ipc/channels";
import { setNotificationState } from "../stores/notification.store";
import { authState } from "../stores/auth.store";
import { settingsState } from "../stores/settings.store";
import { communityState } from "../stores/community.store";
import { commands } from "../ipc/commands";

export async function showSystemNotification(title: string, body: string): Promise<void> {
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (granted) {
      await sendNotification({ title, body });
    }
  } catch (error) {
    console.warn("Failed to show system notification:", error);
  }
}

// Architecture §32 Phase 5 W18 + Phase 7 W25 — when a community-defined
// notification sound (soundboard expression `content_hash`) resolves
// for an incoming message, play it locally. Backend already handled
// DND, quiet hours and rate limiting; this layer is purely "play the
// resolved sound asset, if any". Falls back silently to the bundled
// default sound when the asset isn't cached locally.
function playNotificationSound(communityId: string | undefined, soundRef: string | null | undefined): void {
  if (!settingsState.soundEnabled) return;
  if (!soundRef || !communityId) return;
  const community = communityState.communities[communityId];
  if (!community) return;
  const asset = (community.expressions ?? []).find(
    (expr) => expr.kind === "soundboard" && expr.contentHash === soundRef,
  );
  const dataUrl = asset?.inlineDataUrl;
  if (!dataUrl) return;
  try {
    const audio = new Audio(dataUrl);
    const volume = asset?.soundMeta?.volume;
    audio.volume = typeof volume === "number" ? Math.min(Math.max(volume, 0), 1) : 1.0;
    void audio.play().catch((e) => {
      console.warn("notification sound playback failed:", e);
    });
  } catch (e) {
    console.warn("notification sound playback failed:", e);
  }
}

export function subscribeNotificationHandler(): Promise<UnlistenFn> {
  return subscribeNotificationEvents((event) => {
    switch (event.type) {
      case "messageReceived": {
        // Architecture §32 Phase 7 Week 25 — `soundRef` is the
        // resolved sound override (channel → community → null). The
        // `null` case lets us fall through to the bundled default.
        void showSystemNotification(event.data.title, event.data.body);
        playNotificationSound(event.data.communityId, event.data.soundRef);
        setNotificationState("notifications", (prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            type: "message",
            title: event.data.title,
            body: event.data.body,
            communityId: event.data.communityId,
            channelId: event.data.channelId,
            soundRef: event.data.soundRef,
            timestamp: Date.now(),
            read: false,
          },
        ]);
        setNotificationState("unreadCount", (c) => c + 1);
        break;
      }
      case "systemAlert": {
        void showSystemNotification(event.data.title, event.data.body);
        setNotificationState("notifications", (prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            type: "system",
            title: event.data.title,
            body: event.data.body,
            timestamp: Date.now(),
            read: false,
          },
        ]);
        setNotificationState("unreadCount", (c) => c + 1);
        break;
      }
      case "sessionResetRequested": {
        // P3.3 — peer wants to re-establish the Signal session. Show
        // a confirm dialog with the safety_number so the user verifies
        // the peer's identity out-of-band BEFORE accepting. This is
        // the user-driven side of the safety stance: no auto-process,
        // no creative paths, the user must affirm the safety number.
        const { peerPublicKey, peerDisplayName, safetyNumber } = event.data;
        const accepted = window.confirm(
          `${peerDisplayName} wants to reset the secure session.\n\n` +
            `Safety number: ${safetyNumber}\n\n` +
            `Compare this number with ${peerDisplayName} on a different ` +
            `channel (phone call, in person, separate trusted app) BEFORE ` +
            `accepting. If the numbers don't match, click Cancel — accepting ` +
            `would install an attacker's keys.\n\n` +
            `Accept and re-establish secure session?`,
        );
        if (accepted) {
          void commands.acceptSessionReset(peerPublicKey).catch((e) => {
            console.error("Failed to accept session reset:", e);
          });
        } else {
          void commands.declineSessionReset(peerPublicKey, "user declined").catch((e) => {
            console.error("Failed to decline session reset:", e);
          });
        }
        // Also persist to the notification list so the user sees it in
        // the notifications panel even if they dismissed the modal.
        setNotificationState("notifications", (prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            type: "system",
            title: "Session Reset Request",
            body: `${peerDisplayName} requested a session reset (safety number: ${safetyNumber})`,
            timestamp: Date.now(),
            read: false,
          },
        ]);
        setNotificationState("unreadCount", (c) => c + 1);
        break;
      }
      case "updateAvailable": {
        if (authState.status === "busy") break;
        void showSystemNotification(
          "Update Available",
          `Version ${event.data.version} is available`,
        );
        setNotificationState("notifications", (prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            type: "system",
            title: "Update Available",
            body: `Version ${event.data.version} is available`,
            timestamp: Date.now(),
            read: false,
          },
        ]);
        setNotificationState("unreadCount", (c) => c + 1);
        break;
      }
    }
  });
}
