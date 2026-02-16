import { Component, onMount, onCleanup, createMemo, createSignal, Show } from "solid-js";
import { type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import MessageList from "../components/chat/MessageList";
import MessageInput from "../components/chat/MessageInput";
import TypingIndicator from "../components/chat/TypingIndicator";
import StatusDot from "../components/status/StatusDot";
import VoicePanel from "../components/voice/VoicePanel";
import { chatState, setChatState } from "../stores/chat.store";
import { authState } from "../stores/auth.store";
import type { UserStatus } from "../stores/auth.store";
import { friendsState } from "../stores/friends.store";
import { voiceState } from "../stores/voice.store";
import { handleLoadHistory, handleResetUnread, handleRetrySendMessage } from "../handlers/chat.handlers";
import { handleJoinVoice, handleLeaveVoice } from "../handlers/voice.handlers";
import { subscribeDmChatEvents } from "../handlers/chat-events.handlers";
import { subscribeChatPresenceEvents } from "../handlers/presence-events.handlers";
import { hydrateState } from "../ipc/hydrate";
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

  const [peerStatus, setPeerStatus] = createSignal<UserStatus>(
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

  onMount(() => {
    hydrateState();
    setChatState("activeConversation", peerId);
    handleLoadHistory(peerId, 50);
    handleResetUnread(peerId);

    unlisteners.push(subscribeDmChatEvents(peerId, () => authState.publicKey ?? ""));
    unlisteners.push(subscribeChatPresenceEvents(peerId, setPeerStatus));
  });

  onCleanup(() => {
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
        messages={conversation().messages}
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
