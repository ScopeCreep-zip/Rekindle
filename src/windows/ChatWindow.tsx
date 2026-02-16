import { Component, onMount, onCleanup, createMemo, createSignal, createEffect, Show } from "solid-js";
import { type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import MessageList from "../components/chat/MessageList";
import MessageInput from "../components/chat/MessageInput";
import TypingIndicator from "../components/chat/TypingIndicator";
import StatusDot from "../components/status/StatusDot";
import VoicePanel from "../components/voice/VoicePanel";
import { chatState, setChatState, type Message } from "../stores/chat.store";
import { authState } from "../stores/auth.store";
import { friendsState } from "../stores/friends.store";
import { voiceState } from "../stores/voice.store";
import { handleLoadHistory, handleResetUnread, handleRetrySendMessage } from "../handlers/chat.handlers";
import { handleJoinVoice, handleLeaveVoice } from "../handlers/voice.handlers";
import { subscribeDmChatEvents } from "../handlers/chat-events.handlers";
import { subscribeBuddyListPresenceEvents } from "../handlers/presence-events.handlers";
import { hydrateState } from "../ipc/hydrate";
import { commands } from "../ipc/commands";
import { ICON_PHONE, ICON_HANGUP } from "../icons";

function getPeerFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("peer") ?? "";
}

const ChatWindow: Component = () => {
  const peerId = getPeerFromUrl();

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

  // Use createEffect + createSignal instead of createMemo — guarantees re-run
  // when tracked dependencies change regardless of calling context (DOM event vs
  // Tauri listen callback). The signal is a plain value that For/mapArray reacts
  // to without proxy-comparison issues.
  const [messages, setMessages] = createSignal<Message[]>([]);
  createEffect(() => {
    const convo = chatState.conversations[peerId];
    setMessages(convo ? [...convo.messages] : []);
  });

  const ownName = createMemo(() => {
    return authState.displayName ?? "You";
  });

  const isInCallWithPeer = createMemo(() => {
    return voiceState.isConnected && voiceState.channelId === peerId;
  });

  function handleCallToggle(): void {
    if (isInCallWithPeer()) {
      handleLeaveVoice();
    } else {
      handleJoinVoice(peerId);
    }
  }

  function handleRetry(messageId: number): void {
    handleRetrySendMessage(peerId, messageId);
  }

  const unlisteners: Promise<UnlistenFn>[] = [];
  let refreshInterval: ReturnType<typeof setInterval> | undefined;

  onMount(async () => {
    // Register event listeners FIRST so no events are missed during hydration.
    // subscribeBuddyListPresenceEvents updates the global friendsState store
    // (each Tauri webview has isolated JS context, so we need our own listener).
    // The peerStatus memo reactively reads from that store.
    unlisteners.push(subscribeDmChatEvents(peerId, () => authState.publicKey ?? ""));
    unlisteners.push(subscribeBuddyListPresenceEvents());

    // Await hydration so stores are populated before loading history
    await hydrateState();
    setChatState("activeConversation", peerId);
    await handleLoadHistory(peerId, 50);
    handleResetUnread(peerId);

    // Prepare chat session — ensure fresh route for this peer before sending
    await commands.prepareChatSession(peerId).catch(() => {});

    // Auto-retry any previously failed messages now that route is fresh
    const convo = chatState.conversations[peerId];
    if (convo) {
      const failedMsgs = convo.messages.filter((m) => m.status === "failed");
      for (const msg of failedMsgs) {
        handleRetrySendMessage(peerId, msg.id);
      }
    }

    // Periodic route refresh (catches route rotations during long chats)
    refreshInterval = setInterval(() => {
      commands.prepareChatSession(peerId).catch(() => {});
    }, 60_000);

    // Catch up: sync presence from DHT — the memo will auto-update
    // when emitFriendsPresence updates the global friendsState store.
    await commands.emitFriendsPresence();
  });

  onCleanup(() => {
    if (refreshInterval) clearInterval(refreshInterval);
    for (const p of unlisteners) {
      p.then((unlisten) => unlisten());
    }
  });

  return (
    <div class="app-frame">
      <Titlebar title={`Chat — ${peerName()}`} showMaximize />
      <div class="chat-peer-status">
        <StatusDot status={peerStatus()} />
        <span class="chat-peer-status-label">{peerStatus()}</span>
        <button
          class={`chat-call-btn ${isInCallWithPeer() ? "chat-call-btn-active" : ""}`}
          onClick={handleCallToggle}
          title={isInCallWithPeer() ? "End Call" : "Voice Call"}
        >
          <span class="nf-icon">
            {isInCallWithPeer() ? ICON_HANGUP : ICON_PHONE}
          </span>
        </button>
      </div>
      <MessageList
        messages={messages()}
        ownName={ownName()}
        peerName={peerName()}
        onRetry={handleRetry}
      />
      <Show when={isInCallWithPeer()}>
        <VoicePanel />
      </Show>
      <TypingIndicator isTyping={conversation().isTyping} peerName={peerName()} />
      <MessageInput peerId={peerId} />
    </div>
  );
};

export default ChatWindow;
