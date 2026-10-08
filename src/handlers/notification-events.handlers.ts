import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeNotificationEvents } from "../ipc/channels";
import { setNotificationState } from "../stores/notification.store";
import { settingsState } from "../stores/settings.store";
import { communityState } from "../stores/community.store";
import { callsState } from "../stores/calls.store";
import { queueSessionReset } from "../stores/session-reset.store";

// Wave 12 W12.2 — true when an in-call DND auto-suppression should
// short-circuit message OS notifications / sounds. The user-facing
// modal still surfaces; only the noisy ambient channels are gated.
function inCallSuppress(): boolean {
  return (
    settingsState.inCallDndAutoEnable && callsState.activeCall != null
  );
}

// Architecture §32 Phase 5 W18 + Phase 7 W25 — when a community-defined
// notification sound (soundboard expression `content_hash`) resolves
// for an incoming message, play it locally. Backend already handled
// DND, quiet hours and rate limiting; this layer is purely "play the
// resolved sound asset, if any". Falls back silently to the bundled
// default sound when the asset isn't cached locally.
function playNotificationSound(communityId: string | undefined, soundRef: string | null | undefined): void {
  if (!settingsState.soundEnabled) return;
  if (inCallSuppress()) return;
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
    if ("messageReceived" in event) {
      const data = event.messageReceived;
      // Architecture §32 Phase 7 Week 25 — `soundRef` is the
      // resolved sound override (channel → community → null). The
      // `null` case lets us fall through to the bundled default.
      // The OS notification is the backend's decision
      // (`rekindle_events::notify_policy`); this window plays the
      // community's sound (silenced in a call when the user asked for
      // that) and keeps the inbox row.
      playNotificationSound(data.communityId, data.soundRef);
      setNotificationState("notifications", (prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          type: "message",
          title: data.title,
          body: data.body,
          communityId: data.communityId,
          channelId: data.channelId,
          soundRef: data.soundRef,
          timestamp: Date.now(),
          read: false,
        },
      ]);
      setNotificationState("unreadCount", (c) => c + 1);
    } else if ("systemAlert" in event) {
      const data = event.systemAlert;
      setNotificationState("notifications", (prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          type: "system",
          title: data.title,
          body: data.body,
          timestamp: Date.now(),
          read: false,
        },
      ]);
      setNotificationState("unreadCount", (c) => c + 1);
    } else if ("sessionResetRequested" in event) {
      // P3.3 — peer wants to re-establish the Signal session. The
      // buddy list's SessionResetDialog asks the user to compare the
      // safety number out-of-band and decide explicitly; it cannot be
      // dismissed, so closing it never counts as an answer.
      const { peerPublicKey, peerDisplayName, safetyNumber } =
        event.sessionResetRequested;
      queueSessionReset({ peerPublicKey, peerDisplayName, safetyNumber });
      // Also persist to the notification list.
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
    } else if ("updateAvailable" in event) {
      const { version } = event.updateAvailable;
      setNotificationState("notifications", (prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          type: "system",
          title: "Update Available",
          body: `Version ${version} is available`,
          timestamp: Date.now(),
          read: false,
        },
      ]);
      setNotificationState("unreadCount", (c) => c + 1);
    }
    // `callIncoming` needs nothing here: the backend shows the OS
    // notification, and the call handler drives the ring and modal.
  });
}
