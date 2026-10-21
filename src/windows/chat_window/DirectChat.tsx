import { Component, onMount, onCleanup, createMemo, createSignal, createEffect, Show } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { startEventStream } from "../../ipc/channels";
import Titlebar from "../../components/titlebar/Titlebar";
import MessageList from "../../components/chat/MessageList";
import MessageInput from "../../components/chat/MessageInput";
import TypingIndicator from "../../components/chat/TypingIndicator";
import StatusDot from "../../components/status/StatusDot";
import { chatState, setChatState, type Message } from "../../stores/chat.store";
import { authState } from "../../stores/auth.store";
import { friendsState } from "../../stores/friends.store";
import { callsState } from "../../stores/calls.store";
import { handleLoadHistory, handleResetUnread, handleRetrySendMessage, handleDiscardSendMessage } from "../../actions/chat.actions";
import { handleStartDmCall, handleEndDmCall } from "../../actions/calls.actions";
import { subscribeDmChatEvents } from "../../handlers/chat-events.handlers";
import { subscribeBuddyListPresenceEvents } from "../../handlers/presence-events.handlers";
import { subscribeCallEvents } from "../../handlers/calls.handlers";
import { hydrateState } from "../../stores/hydrate";
import { ICON_PHONE, ICON_VIDEO, ICON_HANGUP, ICON_SHIELD } from "../../icons";
import SafetyNumberPanel from "../../components/profile/SafetyNumberPanel";
import VerifiedBadge from "../../components/profile/VerifiedBadge";
import { useSafetyNumber } from "../../hooks/useSafetyNumber";
import { identityPinMismatchText } from "../../utils/identity_pin";

/// The chat window's 1:1 mode: a conversation with a friend or a
/// community member, with its call controls and typing indicator.
const DirectChat: Component<{ peerId: string }> = (props) => {
  const peerId = props.peerId;

  const peerName = createMemo(() => {
    const friend = friendsState.friends[peerId];
    return friend?.displayName ?? peerId.slice(0, 12) + "...";
  });

  // Reactive memo — tracks the global store, so any presence event that
  // updates friendsState (via subscribeBuddyListPresenceEvents) is reflected
  // immediately without maintaining a parallel local signal.
  const peerStatus = createMemo(() =>
    friendsState.friends[peerId]?.status ?? "offline",
  );

  const conversation = createMemo(() => {
    return chatState.conversations[peerId] ?? {
      peerId,
      messages: [],
      isTyping: false,
      lastRead: 0,
    };
  });

  const [messages, setMessages] = createSignal<Message[]>([]);

  // Their keys are not signed by the identity we know (plan E2.4, B3): the
  // Host refuses every send, so the input says why instead of offering one.
  // Only a friend has a safety number to compare (audit U-9).
  const isFriend = createMemo(() => friendsState.friends[peerId] !== undefined);
  const pinMismatch = createMemo(() => friendsState.friends[peerId]?.identityPinMismatch === true);

  // The peer's key trust (plan E2.3): the header's badge, and the safety
  // number with its mark-verified flow behind the shield button.
  const safety = useSafetyNumber(peerId);
  const [showSafety, setShowSafety] = createSignal(false);

  // Extracted so both createEffect and the direct event listener can call it
  function syncMessages() {
    const convo = chatState.conversations[peerId];
    setMessages(convo ? [...convo.messages] : []);
  }

  // Handles history load, sent messages, and any store-driven updates
  createEffect(syncMessages);

  const ownName = createMemo(() => {
    return authState.displayName ?? "You";
  });

  // W13-fix.1 — read from the 1:1 call store so the active state matches
  // the buddy-list right-click flow. Prior code read voiceState which is
  // the COMMUNITY voice channel state — completely unrelated to 1:1 calls
  // and bypassed the entire CallInvite handshake (start_dm_call →
  // CallInvite → ring → accept).
  const activeCallWithPeer = createMemo(() => {
    const a = callsState.activeCall;
    return a && a.peerKey === peerId ? a : null;
  });
  const outgoingToThisPeer = createMemo(() => {
    const o = callsState.outgoingCall;
    return o && o.peerKey === peerId ? o : null;
  });

  function startCall(video: boolean): void {
    void handleStartDmCall(peerId, peerName(), video);
  }

  function hangup(): void {
    const id = activeCallWithPeer()?.callId ?? outgoingToThisPeer()?.callId;
    if (id) void handleEndDmCall(id);
  }

  function handleRetry(messageId: number): void {
    handleRetrySendMessage(peerId, messageId);
  }

  function handleDiscard(messageId: number): void {
    handleDiscardSendMessage(peerId, messageId);
  }

  const unlisteners: Promise<UnlistenFn>[] = [];
  // Focusing the window reads what it shows (plan E2.5: the read marker
  // follows focus and rendering).
  const onFocus = (): void => handleResetUnread(peerId);

  onMount(async () => {
    // Register event listeners FIRST so no events are missed during hydration.
    // subscribeBuddyListPresenceEvents updates the global friendsState store
    // (each Tauri webview has isolated JS context, so we need our own listener).
    // The peerStatus memo reactively reads from that store.
    unlisteners.push(
      subscribeDmChatEvents(peerId, () => authState.publicKey ?? "", () =>
        queueMicrotask(syncMessages),
      ),
    );
    unlisteners.push(subscribeBuddyListPresenceEvents());
    // Keeps the call buttons in step with a call to this peer.
    unlisteners.push(subscribeCallEvents({ owner: false }));
    void startEventStream(() => void hydrateState());

    // Await hydration so stores are populated before loading history
    await hydrateState();
    setChatState("activeConversation", peerId);
    await handleLoadHistory(peerId, 50);
    handleResetUnread(peerId);
    window.addEventListener("focus", onFocus);
  });

  onCleanup(() => {
    window.removeEventListener("focus", onFocus);
    for (const p of unlisteners) {
      p.then((unlisten) => unlisten());
    }
  });

  return (
    <>
      <Titlebar title={`Chat — ${peerName()}`} showMaximize />
      <div class="chat-peer-status">
        <StatusDot status={peerStatus()} />
        <span class="chat-peer-status-label">{peerStatus()}</span>
        <VerifiedBadge view={safety.view()} />
        <Show when={isFriend()}>
          <button
            class="chat-safety-btn"
            onClick={() => setShowSafety(!showSafety())}
            title="Safety number"
            aria-label={`Safety number with ${peerName()}`}
            aria-expanded={showSafety()}
            aria-controls={showSafety() ? "chat-safety-number" : undefined}
          >
            <span class="nf-icon" aria-hidden="true">{ICON_SHIELD}</span>
          </button>
        </Show>
        {/* W13-fix.1 — voice + video buttons matching the buddy-list
         *  right-click menu. Both go through the start_dm_call →
         *  CallInvite handshake. When a call is active with this peer,
         *  the voice button collapses into a hangup. */}
        <Show
          when={!activeCallWithPeer() && !outgoingToThisPeer()}
          fallback={
            <button
              class="chat-call-btn chat-call-btn-active"
              onClick={hangup}
              title="End call"
              aria-label={`End call with ${peerName()}`}
            >
              <span class="nf-icon" aria-hidden="true">{ICON_HANGUP}</span>
            </button>
          }
        >
          <button
            class="chat-call-btn"
            onClick={() => startCall(false)}
            title="Voice call"
            aria-label={`Start voice call with ${peerName()}`}
          >
            <span class="nf-icon" aria-hidden="true">{ICON_PHONE}</span>
          </button>
          <button
            class="chat-call-btn"
            onClick={() => startCall(true)}
            title="Video call"
            aria-label={`Start video call with ${peerName()}`}
          >
            <span class="nf-icon" aria-hidden="true">{ICON_VIDEO}</span>
          </button>
        </Show>
      </div>
      <Show when={showSafety() && isFriend()}>
        <div id="chat-safety-number" class="chat-safety-number">
          <SafetyNumberPanel peerKey={peerId} view={safety.view()} onChanged={safety.reload} />
        </div>
      </Show>
      <div id="main-content" tabindex="-1" class="window-main">
      <MessageList
        messages={messages()}
        ownName={ownName()}
        peerName={peerName()}
        historyGaps={conversation().historyGaps}
        onRetry={handleRetry}
        onDiscard={handleDiscard}
      />
      {/* A connected call's controls (timer, mute, hangup, video) live in
       *  its own call window, which the backend opens on connect. */}
      <TypingIndicator isTyping={conversation().isTyping} peerName={peerName()} />
      <MessageInput
        peerId={peerId}
        disabled={pinMismatch()}
        disabledMessage={identityPinMismatchText(peerName())}
      />
      </div>
    </>
  );
};

export default DirectChat;
