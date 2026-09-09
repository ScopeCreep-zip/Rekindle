// Voice user actions — join, leave, mute, deafen, speak requests.
//
// Split from `handlers/voice.handlers.ts`, which also registers the two
// voice-event subscriptions. A mute button and an app-start event
// subscription are different tiers; sharing a module made every
// component that renders a call control import the subscription wiring.

import type { UnlistenFn } from "@tauri-apps/api/event";
import { commands } from "../ipc/commands";
import { subscribeVoiceEvents } from "../ipc/channels";
import { callType, sessionKey } from "../ipc/channels/voice_events";
import { friendsState } from "../stores/friends.store";
import { voiceState, setVoiceState } from "../stores/voice.store";
import { communityState } from "../stores/community.store";
import { addToast } from "../stores/toast.store";
import { probeAndReportLocalVideoCapabilities } from "./video.actions";

let voiceEventUnlisten: UnlistenFn | null = null;

/** Subscribe to voice events from the backend and update the store. */
export async function initVoiceEventListener(): Promise<UnlistenFn> {
  return subscribeVoiceEvents((event) => {
    if ("localJoined" in event) {
      // Backend-authoritative join state — every frontend (Tauri GUI, CLI,
      // future TUI) mirrors the same call shape from this single event.
      // Fixes C1: VideoCallPanel's <Show> gate at CommunityWindow.tsx:731-744
      // requires activeCallType === "community" but the prior frontend-only
      // set ran AFTER the gate evaluated, so the panel never mounted.
      //
      // `activeCallType` is now derived from the scope rather than sent
      // as its own string, so the two can no longer disagree.
      const { scope } = event.localJoined;
      setVoiceState({
        isConnected: true,
        channelId: sessionKey(scope),
        activeCallType: callType(scope),
      });
    } else if ("joined" in event) {
      const { pseudonym, displayName } = event.joined;
      setVoiceState("participants", (prev) => [
        ...prev.filter((p) => p.publicKey !== pseudonym),
        {
          publicKey: pseudonym,
          displayName: displayName ?? pseudonym,
          isMuted: false,
          isSpeaking: false,
        },
      ]);
    } else if ("left" in event) {
      const { pseudonym } = event.left;
      setVoiceState("participants", (prev) =>
        prev.filter((p) => p.publicKey !== pseudonym),
      );
    } else if ("speakingChanged" in event) {
      const { pseudonym, speaking } = event.speakingChanged;
      setVoiceState(
        "participants",
        (p) => p.publicKey === pseudonym,
        "isSpeaking",
        speaking,
      );
    } else if ("muteChanged" in event) {
      const { targetPseudonym, muted } = event.muteChanged;
      setVoiceState(
        "participants",
        (p) => p.publicKey === targetPseudonym,
        "isMuted",
        muted,
      );
    } else if ("connectionQuality" in event) {
      const q = event.connectionQuality;
      setVoiceState("connectionQuality", q.quality);
      setVoiceState("rxOverflowDrops", q.rxOverflowDrops);
      setVoiceState("rxLateDrops", q.rxLateDrops);
      setVoiceState("rxMekDrops", q.rxMekDrops);
      setVoiceState("ingressDrops", q.ingressDrops);
    } else if ("deviceChanged" in event) {
      setVoiceState("deviceChangeCount", (prev) => prev + 1);
    } else if ("packetsDropped" in event) {
      // W14.4 — backend tells us audio packets were dropped over
      // the last 1 s. Toast so the user sees an objective signal
      // ("audio interrupted") rather than confused silence. Backend
      // already logged details at info!/warn!.
      const { count, reason } = event.packetsDropped;
      addToast(`Voice packets dropped: ${count} (${reason})`, "error");
    }
    // modeChanged / deafenChanged / rosterUpdated are community-channel
    // signals handled by the community window, not this session store.
  });
}

export async function handleJoinVoice(channelId: string, communityId?: string): Promise<void> {
  try {
    // Lazy one-shot WebCodecs probe — first call context is the right
    // time to learn the WebView's encoder/decoder reach. Deliberately
    // NOT awaited: caps that land after LocalJoined recompute + re-emit
    // the session config, and the probe must never delay joining audio.
    // It cannot run earlier (login path): a cold WebCodecs call aborts
    // the web process on WebKitGTK 2.52.3 + GStreamer 1.24 (Ubuntu /
    // Pop!_OS 24.04) — see handlers/video.handlers.ts.
    probeAndReportLocalVideoCapabilities().catch((e) => {
      console.error("WebCodecs capability probe failed:", e);
    });

    // Subscribe BEFORE the command fires so the LocalJoined event the backend
    // emits during start_session reaches us — late subscription would miss it.
    if (!voiceEventUnlisten) {
      voiceEventUnlisten = await initVoiceEventListener();
    }
    await commands.joinVoiceChannel(channelId, communityId);
    // Community joins start the three-way handshake: VoiceJoin is out,
    // nobody has seen us yet. The backend's voiceJoinHandshake events
    // advance this to "seen"/"connected".
    if (communityId) {
      setVoiceState("joinHandshake", "announced");
      // Media-ready resets with the new session; the backend emits the
      // first voiceMediaReady transition once the slot is seeded.
      setVoiceState("mediaReady", null);
    }
    // Backend emits VoiceEvent::LocalJoined which the listener mirrors into
    // voiceState (isConnected + channelId + activeCallType). No manual set
    // here — that was the C1 bug: prior code set isConnected/channelId but
    // not activeCallType, so the <Show> gate at CommunityWindow.tsx:731-744
    // never opened and VideoCallPanel never mounted.

    // Publish our Voice session location so peers see "in 🔊 channel" on the
    // roster. The dedicated join button calls stopPropagation, so the
    // channel-row select never fires for it — set the location here so both
    // entry points (row select and join button) announce the move.
    if (communityId) {
      commands.setActiveChannel(communityId, channelId, "voice").catch((e) => {
        console.warn("Failed to publish voice location:", e);
      });
    }
  } catch (e) {
    console.error("Failed to join voice:", e);
  }
}

export async function handleLeaveVoice(): Promise<void> {
  // Capture the call context before leaveVoice resets voiceState so we know
  // whether to clear a community Voice location.
  const wasCommunityCall = voiceState.activeCallType === "community";
  const communityId = communityState.activeCommunity;
  try {
    await commands.leaveVoice();

    // Unsubscribe from voice events
    if (voiceEventUnlisten) {
      voiceEventUnlisten();
      voiceEventUnlisten = null;
    }

    setVoiceState({
      isConnected: false,
      channelId: null,
      participants: [],
      connectionQuality: "good",
      activeCallType: null,
      joinHandshake: null,
      mediaReady: null,
    });

    // Clear our Voice session location so the roster stops showing us in the
    // channel we just left. The next text-channel select re-asserts a Text
    // location; until then we show as focused nowhere (single-field model).
    if (wasCommunityCall && communityId) {
      commands.setActiveChannel(communityId, null, "voice").catch((e) => {
        console.warn("Failed to clear voice location:", e);
      });
    }
  } catch (e) {
    console.error("Failed to leave voice:", e);
  }
}

export async function handleToggleMute(): Promise<void> {
  try {
    const newMuted = !voiceState.isMuted;
    await commands.setMute(newMuted);
    setVoiceState("isMuted", newMuted);
  } catch (e) {
    console.error("Failed to toggle mute:", e);
  }
}

export async function handleToggleDeafen(): Promise<void> {
  try {
    const newDeafened = !voiceState.isDeafened;
    await commands.setDeafen(newDeafened);
    setVoiceState("isDeafened", newDeafened);
  } catch (e) {
    console.error("Failed to toggle deafen:", e);
  }
}

export async function handleRequestToSpeak(
  communityId: string,
  channelId: string,
): Promise<void> {
  try {
    await commands.requestToSpeak(communityId, channelId);
  } catch (e) {
    console.error("Failed to request to speak:", e);
  }
}

export async function handleRespondToSpeakRequest(
  communityId: string,
  channelId: string,
  requesterPseudonym: string,
  granted: boolean,
): Promise<void> {
  try {
    await commands.respondToSpeakRequest(
      communityId,
      channelId,
      requesterPseudonym,
      granted,
    );
  } catch (e) {
    console.error("Failed to respond to speak request:", e);
  }
}
