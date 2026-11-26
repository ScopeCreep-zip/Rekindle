import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../../ipc/channels";
import { produce } from "solid-js/store";
import { setCallsState } from "../../stores/calls.store";
import { reduceCallEvent, reduceMissedCalls, type CallEffect } from "../../stores/calls.reduce";
import { errorMessage } from "../../utils/error";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import { settingsState } from "../../stores/settings.store";
import { setNotificationState } from "../../stores/notification.store";
import { setVoiceState } from "../../stores/voice.store";
import { friendsState } from "../../stores/friends.store";
import { authState } from "../../stores/auth.store";
import { playBusyTone, playIncomingRing, playOutgoingRingback } from "../../utils/ringtone";
import { startRing, stopActiveRing } from "../../actions/calls_ring";

/// Wave 12 W12.8 — push a `missed_call` row into the notification inbox
/// so the user sees a Call-back / Send Message action even after the
/// call has terminated. Looks up the friend's display name from the
/// friends store; falls back to a truncated pubkey otherwise.
function pushMissedCallNotification(
  callId: string,
  peerKey: string,
  kind: "audio" | "video",
  outgoing: boolean,
): void {
  const friend = friendsState.friends[peerKey];
  const name = friend?.displayName ?? peerKey.slice(0, 12) + "…";
  setNotificationState("notifications", (prev) => [
    ...prev,
    {
      id: crypto.randomUUID(),
      type: "missed_call",
      title: outgoing ? "Call not answered" : "Missed call",
      body: `${name} (${kind})`,
      timestamp: Date.now(),
      read: false,
      callId,
      peerKey,
      callKind: kind,
    },
  ]);
}

/// Wires the call-signalling family into `calls.store`. Components read
/// from the store; nothing else owns call state.
///
/// Every window that shows a call (buddy list, chat, DM, call) keeps its
/// store current. Only the `owner` — the buddy list — acts on the user's
/// behalf: ringing, toasts, missed-call rows, opening the conversation.
/// Each webview is a separate JS context, so without this split every
/// open window rang and toasted.
///
/// The backend collapsed four 1:1/group variant pairs into one each, so
/// `incoming`, `connected` and `ended` now branch on their payload
/// rather than on which of two events arrived. `ended` covers both,
/// which is why it clears the group slot too — the old `callEnded` arm
/// did not, and a 1:1 `CallEnd` for a group call left the panel up.
export function subscribeCallEvents({ owner }: { owner: boolean }): Promise<UnlistenFn> {
  /// Run a user-facing side effect only in the owning window.
  const asOwner = (effect: () => void): void => {
    if (owner) effect();
  };
  const toast = (message: string): void => asOwner(() => addToast(message, "info"));
  return subscribeChatEvents((event) => {
    if ("channelMessage" in event) {
      const msg = event.channelMessage;
      if ("conversationFocusRequested" in msg) {
        // W14.3 — backend asked us to surface the conversation. Tauri
        // opens/focuses the ChatWindow. The "when" decision (after
        // CallInvite send / after Accept resolution) lives in the
        // backend; we just navigate.
        if (owner) void commands.openChatWindow(msg.conversationFocusRequested.peerKey);
      }
      return;
    }

    if (!("call" in event)) return;
    // The store changes only through its reducers; what the event asks of
    // the user (ring, toast, missed-call row, the conversation forward)
    // runs here, in the owning window, the camera policy in every window.
    let effects: CallEffect[] = [];
    const ctx = { nowMs: Date.now(), selfKey: authState.publicKey ?? "", selfName: authState.displayName ?? "" };
    setCallsState(produce((s) => (effects = reduceCallEvent(s, event.call, ctx))));
    for (const effect of effects) runEffect(effect);
  });

  function runEffect(effect: CallEffect): void {
    switch (effect.kind) {
      case "cameraOn":
        setVoiceState("cameraOn", true);
        return;
      case "ring":
        if (!settingsState.ringtoneEnabled) return;
        asOwner(() =>
          startRing(
            effect.tone === "incoming"
              ? playIncomingRing({ volume: settingsState.ringtoneVolume })
              : effect.tone === "ringback"
                ? playOutgoingRingback({ volume: settingsState.ringtoneVolume })
                : playBusyTone({ volume: settingsState.ringtoneVolume * 0.6 }),
          ),
        );
        return;
      case "stopRing":
        asOwner(stopActiveRing);
        return;
      case "toast":
        toast(effect.text);
        return;
      case "missedNotification":
        asOwner(() => pushMissedCallNotification(effect.callId, effect.peerKey, effect.callKind, effect.outgoing));
        return;
      case "refreshMissed":
        asOwner(() => void refreshMissedCalls());
        return;
      case "focusConversation":
        asOwner(() => void commands.openChatWindow(effect.peerKey));
        return;
    }
  }
}

export async function refreshMissedCalls(): Promise<void> {
  try {
    const rows = await commands.getMissedCalls();
    setCallsState(produce((s) => reduceMissedCalls(s, rows)));
  } catch (e) {
    addToast(errorMessage(e), "error");
  }
}
