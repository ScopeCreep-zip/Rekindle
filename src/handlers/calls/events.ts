import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../../ipc/channels";
import { isLegacy } from "../../ipc/channels/chat_events";
import { callsState, setCallsState, type CallEntry } from "../../stores/calls.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import { settingsState } from "../../stores/settings.store";
import { setNotificationState } from "../../stores/notification.store";
import { setVoiceState } from "../../stores/voice.store";
import { friendsState } from "../../stores/friends.store";
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
  setNotificationState("unreadCount", (c) => c + 1);
}

/// Wires the call-signalling family into `calls.store`. Components read
/// from the store; nothing else owns call state.
///
/// The backend collapsed four 1:1/group variant pairs into one each, so
/// `incoming`, `connected` and `ended` now branch on their payload
/// rather than on which of two events arrived. `ended` covers both,
/// which is why it clears the group slot too — the old `callEnded` arm
/// did not, and a 1:1 `CallEnd` for a group call left the panel up.
export function subscribeCallEvents(): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    if (isLegacy(event)) return;

    if ("channelMessage" in event) {
      const msg = event.channelMessage;
      if ("conversationFocusRequested" in msg) {
        // W14.3 — backend asked us to surface the conversation. Tauri
        // opens/focuses the ChatWindow. The "when" decision (after
        // CallInvite send / after Accept resolution) lives in the
        // backend; we just navigate.
        const { peerKey, displayName } = msg.conversationFocusRequested;
        void commands.openChatWindow(peerKey, displayName);
      }
      return;
    }

    if (!("call" in event)) return;
    const call = event.call;

    if ("started" in call) {
      // W15.6 — backend-emitted on start_dm_call. Seed outgoingCall
      // store from authoritative backend payload (no local clock
      // arithmetic) and start the synthesized ringback. CLI/TUI
      // would react identically (print "Calling X…" + terminal
      // bell). Per feedback_backend_owns_policy.md.
      const { callId, kind, peerKey, peerDisplayName, expiresAtMs } = call.started;
      const seed: CallEntry = {
        callId,
        peerKey,
        displayName: peerDisplayName,
        kind: kind as "audio" | "video",
        expiresAtMs,
        startedAtMs: Date.now(),
        status: "calling",
      };
      setCallsState("outgoingCall", seed);
      if (settingsState.ringtoneEnabled) {
        startRing(playOutgoingRingback({ volume: settingsState.ringtoneVolume }));
      }
      return;
    }

    if ("incoming" in call) {
      const { callId, from, displayName, kind, participants, isGroup, expiresAtMs } =
        call.incoming;

      if (isGroup) {
        // Wave 12 W12.9 — incoming group call. Push into the queue;
        // GroupCallPanel reads incomingGroupCalls[0].
        setCallsState("incomingGroupCalls", (prev) => [
          ...prev,
          {
            callId,
            initiatorKey: from,
            displayName,
            kind: kind as "audio" | "video",
            participants,
            accepted: [],
            startedAtMs: Date.now(),
            expiresAtMs,
          },
        ]);
        // W12-fix.B — always ring (background or foreground). The
        // window-focus path is handled by the backend bringing the
        // window forward (W12-fix.C); the ring is the audio cue.
        if (!settingsState.ringtoneEnabled) return;
        startRing(playIncomingRing({ volume: settingsState.ringtoneVolume }));
        return;
      }

      const entry: CallEntry = {
        callId,
        peerKey: from,
        displayName,
        kind: kind as "audio" | "video",
        expiresAtMs,
        startedAtMs: Date.now(),
      };
      // Queue if multiple offers race in — IncomingCallModal shows
      // the head of the list.
      setCallsState("incomingCalls", (prev) => [...prev, entry]);
      // W12-fix.B — ring even when the window is hidden / unfocused;
      // hearing the call is the WHOLE point. Web Audio plays from
      // background webviews fine. Multi-window overlap is acceptable
      // (you'll hear the ring louder); the alternative — silent ring
      // when the window isn't focused — is unusable.
      // - Already-in-call gets the call-waiting beep instead of a full
      //   ring (Discord/Telegram convention).
      // - User can disable ringtone entirely via settingsState.
      if (!settingsState.ringtoneEnabled) return;
      if (callsState.activeCall != null) {
        startRing(playBusyTone({ volume: settingsState.ringtoneVolume * 0.6 }));
      } else {
        startRing(playIncomingRing({ volume: settingsState.ringtoneVolume }));
      }
      return;
    }

    if ("connected" in call) {
      const { callId, direct } = call.connected;
      stopActiveRing();

      if (direct == null) {
        // Wave 12 W12.9 — promote a group call to active. A group call
        // has no single peer, which is exactly what `direct: null` says.
        const head = callsState.incomingGroupCalls.find((c) => c.callId === callId);
        if (head) {
          setCallsState("activeGroupCall", { ...head, accepted: [head.initiatorKey] });
          setCallsState("incomingGroupCalls", (p) => p.filter((c) => c.callId !== callId));
        }
        return;
      }

      // W13.11 — fire-and-forget signaling guarantees the backend's
      // start_dm_call returns synchronously BEFORE any connected
      // event can fire. Match by exact callId.
      const out = callsState.outgoingCall;
      if (out && out.callId === callId) {
        setCallsState("activeCall", out);
        setCallsState("outgoingCall", null);
      } else {
        const idx = callsState.incomingCalls.findIndex((c) => c.callId === callId);
        if (idx >= 0) {
          const entry = callsState.incomingCalls[idx];
          setCallsState("activeCall", entry);
          setCallsState("incomingCalls", (prev) => prev.filter((_, i) => i !== idx));
        }
      }
      // W14.2 — backend told us the camera policy. Tauri reacts by
      // starting capture; the policy itself lives in the backend's
      // event emit (not here).
      if (direct.expectedLocalCamera) {
        setVoiceState("cameraOn", true);
      }
      return;
    }

    if ("ringing" in call) {
      // W13 — receiver acknowledged our invite and is ringing the
      // user. Flip the OutgoingCallPanel label from "Calling…" to
      // "Ringing…" via a status field on the entry.
      const out = callsState.outgoingCall;
      if (out && out.callId === call.ringing.callId) {
        setCallsState("outgoingCall", "status", "ringing");
      }
      return;
    }

    if ("timedOut" in call) {
      const { callId } = call.timedOut;
      stopActiveRing();
      const out = callsState.outgoingCall;
      if (out?.callId === callId) {
        pushMissedCallNotification(out.callId, out.peerKey, out.kind, true);
        setCallsState("outgoingCall", null);
        addToast("Call timed out — no answer", "info");
      }
      // Refresh missed list — the backend wrote a row for the
      // local user so the badge ticks up.
      void refreshMissedCalls();
      return;
    }

    if ("missed" in call) {
      const { callId } = call.missed;
      stopActiveRing();
      const idx = callsState.incomingCalls.findIndex((c) => c.callId === callId);
      if (idx >= 0) {
        const entry = callsState.incomingCalls[idx];
        pushMissedCallNotification(entry.callId, entry.peerKey, entry.kind, false);
        setCallsState("incomingCalls", (prev) => prev.filter((_, i) => i !== idx));
      }
      void refreshMissedCalls();
      return;
    }

    if ("declined" in call) {
      const { callId, reason } = call.declined;
      stopActiveRing();
      if (callsState.outgoingCall?.callId === callId) {
        setCallsState("outgoingCall", null);
      }
      addToast(reason ? `Call declined: ${reason}` : "Call declined", "info");
      return;
    }

    if ("participantJoined" in call) {
      const { callId, participantPubkey } = call.participantJoined;
      const cur = callsState.activeGroupCall;
      if (cur && cur.callId === callId && !cur.accepted.includes(participantPubkey)) {
        setCallsState("activeGroupCall", "accepted", (a) => [...a, participantPubkey]);
      }
      return;
    }

    if ("participantLeft" in call) {
      const { callId, participantPubkey } = call.participantLeft;
      const cur = callsState.activeGroupCall;
      if (cur && cur.callId === callId) {
        setCallsState("activeGroupCall", "accepted", (a) =>
          a.filter((p) => p !== participantPubkey),
        );
      }
      return;
    }

    if ("reactionReceived" in call) {
      // Wave 12 W12.11 — peer fired an emoji reaction. Push to the
      // recent-reactions list with a fresh id; ReactionFloater
      // animates and clears after the float window.
      const { callId, sender, emoji, timestampMs } = call.reactionReceived;
      if (callsState.activeCall?.callId !== callId) return;
      setCallsState("recentReactions", (prev) => [
        ...prev,
        { id: crypto.randomUUID(), emoji, sender, timestampMs },
      ]);
      return;
    }

    if ("mediaStateChanged" in call) {
      // Wave 12 W12.6 — peer flipped audio/video/screen. Drop if our
      // local last-update is newer (last-write-wins per timestamp).
      const { callId, audio, video, screen, timestampMs } = call.mediaStateChanged;
      const updateSlot = (
        slot: "activeCall" | "outgoingCall",
        entry: CallEntry | null,
      ): void => {
        if (entry == null || entry.callId !== callId) return;
        const cur = entry.peerMediaState;
        if (cur && cur.timestampMs >= timestampMs) return;
        setCallsState(slot, "peerMediaState", { audio, video, screen, timestampMs });
      };
      updateSlot("activeCall", callsState.activeCall);
      updateSlot("outgoingCall", callsState.outgoingCall);
      const idx = callsState.incomingCalls.findIndex((c) => c.callId === callId);
      if (idx >= 0) {
        const cur = callsState.incomingCalls[idx].peerMediaState;
        if (!cur || cur.timestampMs < timestampMs) {
          setCallsState("incomingCalls", idx, "peerMediaState", {
            audio,
            video,
            screen,
            timestampMs,
          });
        }
      }
      return;
    }

    if ("ended" in call) {
      // Hangup / cancel / remote-end — fires from end_dm_call (local
      // emit) and from process_envelope::CallEnd (peer's CallEnd
      // payload). W15.1 — clear ALL call slots that might reference
      // this callId, not just activeCall + outgoingCall.
      // Cancel-while-ringing path: peer's CallEnd arrives while we're
      // still in IncomingCall state — without filtering incomingCalls,
      // the IncomingCallModal stays zombie because head() still
      // returns the entry.
      const { callId, reason } = call.ended;
      stopActiveRing();
      if (callsState.activeCall?.callId === callId) {
        setCallsState("activeCall", null);
      }
      if (callsState.outgoingCall?.callId === callId) {
        setCallsState("outgoingCall", null);
      }
      // Group slots too: one `ended` now covers both call shapes.
      if (callsState.activeGroupCall?.callId === callId) {
        setCallsState("activeGroupCall", null);
      }
      setCallsState("incomingGroupCalls", (p) => p.filter((c) => c.callId !== callId));
      setCallsState("incomingCalls", (prev) => prev.filter((c) => c.callId !== callId));
      if (reason) {
        addToast(`Call ended: ${reason}`, "info");
      }
    }
  });
}

export async function refreshMissedCalls(): Promise<void> {
  try {
    const rows = await commands.getMissedCalls();
    setCallsState("missed", rows.map((r) => ({
      callId: r.callId,
      peerKey: r.peerKey,
      kind: r.kind,
      expiredAt: r.expiredAt,
    })));
  } catch (e) {
    console.error("Failed to refresh missed calls:", e);
  }
}
