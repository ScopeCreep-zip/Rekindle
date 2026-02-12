import { Component, createSignal, createMemo, createEffect, Show, onMount, onCleanup } from "solid-js";
import { type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import CommunityList from "../components/community/CommunityList";
import ChannelList from "../components/community/ChannelList";
import MemberList from "../components/community/MemberList";
import MessageList from "../components/chat/MessageList";
import MessageInput from "../components/chat/MessageInput";
import VoicePanel from "../components/voice/VoicePanel";
import CreateCommunityModal from "../components/community/CreateCommunityModal";
import CreateChannelModal from "../components/community/CreateChannelModal";
import JoinCommunityModal from "../components/community/JoinCommunityModal";
import { communityState, setCommunityState } from "../stores/community.store";
import { authState } from "../stores/auth.store";
import { voiceState, setVoiceState } from "../stores/voice.store";
import { subscribeChatEvents, subscribeVoiceEvents, subscribePresenceEvents } from "../ipc/channels";
import { hydrateState } from "../ipc/hydrate";
import {
  handleLoadChannelMessages,
  handleSendChannelMessage,
  handleSelectCommunity as storeSyncSelectCommunity,
  handleSelectChannel as storeSyncSelectChannel,
  handleLeaveCommunity,
} from "../handlers/community.handlers";
import { handleJoinVoice } from "../handlers/voice.handlers";
import type { ChatEvent, VoiceEvent, PresenceEvent } from "../ipc/channels";
import type { Message } from "../stores/chat.store";

function handleVoiceEvent(event: VoiceEvent): void {
  switch (event.type) {
    case "UserJoined": {
      setVoiceState("participants", (prev) => [
        ...prev,
        {
          publicKey: event.data.publicKey,
          displayName: event.data.displayName,
          isMuted: false,
          isSpeaking: false,
        },
      ]);
      break;
    }
    case "UserLeft": {
      setVoiceState("participants", (prev) =>
        prev.filter((p) => p.publicKey !== event.data.publicKey),
      );
      break;
    }
    case "UserSpeaking": {
      setVoiceState("participants", (p) => p.publicKey === event.data.publicKey, "isSpeaking", event.data.speaking);
      break;
    }
    case "UserMuted": {
      setVoiceState("participants", (p) => p.publicKey === event.data.publicKey, "isMuted", event.data.muted);
      break;
    }
    case "ConnectionQuality": {
      break;
    }
  }
}

const CommunityWindow: Component = () => {
  function getCommunityFromUrl(): string {
    const params = new URLSearchParams(window.location.search);
    return params.get("id") ?? "";
  }

  const [selectedCommunityId, setSelectedCommunityId] = createSignal(getCommunityFromUrl());
  const [selectedChannelId, setSelectedChannelId] = createSignal<string>("");
  const [showCreateCommunity, setShowCreateCommunity] = createSignal(false);
  const [showCreateChannel, setShowCreateChannel] = createSignal(false);
  const [showJoinCommunity, setShowJoinCommunity] = createSignal(false);

  const activeCommunity = createMemo(() => {
    const id = selectedCommunityId();
    return id ? communityState.communities[id] : undefined;
  });

  const activeChannel = createMemo(() => {
    const community = activeCommunity();
    const channelId = selectedChannelId();
    if (!community || !channelId) return undefined;
    return community.channels.find((c) => c.id === channelId);
  });

  const channelMessages = createMemo((): Message[] => {
    const channelId = selectedChannelId();
    if (!channelId) return [];
    return communityState.channelMessages[channelId] ?? [];
  });

  const myRole = createMemo((): string | null => {
    const community = activeCommunity();
    if (!community) return null;
    const me = community.members.find((m) => m.publicKey === authState.publicKey);
    return me?.role ?? null;
  });

  // Load messages when channel changes
  createEffect(() => {
    const channelId = selectedChannelId();
    if (channelId) {
      handleLoadChannelMessages(channelId, 50);
    }
  });

  function handleSelectCommunity(id: string) {
    setSelectedCommunityId(id);
    storeSyncSelectCommunity(id);
    const community = communityState.communities[id];
    if (community?.channels.length) {
      const firstText = community.channels.find((c) => c.type === "text");
      if (firstText) {
        setSelectedChannelId(firstText.id);
        storeSyncSelectChannel(firstText.id);
      }
    }
  }

  function handleSelectChannel(id: string) {
    setSelectedChannelId(id);
    storeSyncSelectChannel(id);
  }

  const unlisteners: Promise<UnlistenFn>[] = [];

  onMount(async () => {
    await hydrateState();

    // After hydration, auto-select community from URL or pick the first available
    const urlId = getCommunityFromUrl();
    if (urlId && communityState.communities[urlId]) {
      handleSelectCommunity(urlId);
    } else {
      const ids = Object.keys(communityState.communities);
      if (ids.length > 0) {
        handleSelectCommunity(ids[0]);
      }
    }

    unlisteners.push(subscribeVoiceEvents(handleVoiceEvent));

    // Subscribe to presence events so community member status updates in real time
    unlisteners.push(subscribePresenceEvents((event: PresenceEvent) => {
      const key =
        event.type === "FriendOnline" || event.type === "FriendOffline"
          ? event.data.publicKey
          : event.type === "StatusChanged"
            ? event.data.publicKey
            : null;
      if (!key) return;
      const newStatus =
        event.type === "FriendOnline"
          ? "online"
          : event.type === "FriendOffline"
            ? "offline"
            : event.type === "StatusChanged"
              ? event.data.status
              : null;
      if (!newStatus) return;
      // Update member status in all communities
      for (const communityId of Object.keys(communityState.communities)) {
        const community = communityState.communities[communityId];
        const memberIdx = community.members.findIndex((m) => m.publicKey === key);
        if (memberIdx >= 0) {
          setCommunityState("communities", communityId, "members", memberIdx, "status", newStatus);
        }
      }
    }));

    // Subscribe to chat events for incoming channel messages from other users.
    // Our own messages are handled via optimistic insert in handleSendChannelMessage.
    unlisteners.push(subscribeChatEvents((event: ChatEvent) => {
      if (event.type === "MessageReceived") {
        // Skip our own messages — already added optimistically
        if (event.data.from === (authState.publicKey ?? "")) return;
        const channelId = event.data.conversationId;
        const message: Message = {
          id: Date.now(),
          senderId: event.data.from,
          body: event.data.body,
          timestamp: event.data.timestamp,
          isOwn: false,
        };
        const existing = communityState.channelMessages[channelId];
        if (existing) {
          setCommunityState("channelMessages", channelId, (msgs) => [
            ...msgs,
            message,
          ]);
        } else {
          setCommunityState("channelMessages", channelId, [message]);
        }
      }
    }));
  });

  onCleanup(() => {
    for (const p of unlisteners) {
      p.then((unlisten) => unlisten());
    }
  });

  return (
    <div class="app-frame">
      <Titlebar title={activeCommunity()?.name ?? "Community"} showMaximize />
      <div class="community-layout">
        {/* Left sidebar: community list + channel list */}
        <div class="community-sidebar">
          <div class="community-sidebar-header">
            Communities
            <span class="header-btn-group">
              <button
                class="action-bar-btn header-add-btn"
                onClick={() => setShowJoinCommunity(true)}
                title="Join Community"
              >
                &gt;
              </button>
              <button
                class="action-bar-btn header-add-btn"
                onClick={() => setShowCreateCommunity(true)}
                title="Create Community"
              >
                +
              </button>
            </span>
          </div>
          <CommunityList
            selectedId={selectedCommunityId()}
            onSelect={handleSelectCommunity}
          />
          <Show when={activeCommunity()}>
            <div class="community-sidebar-header">
              Channels
              <button
                class="action-bar-btn header-add-btn"
                onClick={() => setShowCreateChannel(true)}
              >
                +
              </button>
            </div>
            <ChannelList
              channels={activeCommunity()!.channels}
              selectedId={selectedChannelId()}
              onSelect={handleSelectChannel}
              onVoiceJoin={handleJoinVoice}
            />
          </Show>
          <Show when={voiceState.isConnected}>
            <VoicePanel />
          </Show>
          <Show when={activeCommunity()}>
            <div class="community-header-actions">
              <button
                class="community-leave-btn"
                onClick={() => handleLeaveCommunity(selectedCommunityId())}
              >
                Leave Community
              </button>
            </div>
          </Show>
        </div>

        {/* Main content: channel header + messages */}
        <div class="community-main">
          <Show when={activeChannel()} fallback={
            <div class="empty-placeholder">
              <div class="empty-placeholder-title">Select a channel</div>
              <div class="empty-placeholder-subtitle">Choose a community and channel to start chatting</div>
            </div>
          }>
            <div class="community-channel-header">
              <span class="community-channel-header-icon">#</span>
              {activeChannel()!.name}
            </div>
            <MessageList
              messages={channelMessages()}
              ownName={authState.displayName ?? "You"}
              peerName="Channel"
            />
            <MessageInput peerId={selectedChannelId()} onSend={handleSendChannelMessage} />
          </Show>
        </div>

        {/* Right sidebar: member list */}
        <Show when={activeCommunity()}>
          <div class="member-sidebar">
            <MemberList
              members={activeCommunity()!.members}
              communityId={selectedCommunityId()}
              myRole={myRole()}
            />
          </div>
        </Show>
      </div>

      <CreateCommunityModal
        isOpen={showCreateCommunity()}
        onClose={() => setShowCreateCommunity(false)}
      />
      <JoinCommunityModal
        isOpen={showJoinCommunity()}
        onClose={() => setShowJoinCommunity(false)}
      />
      <Show when={activeCommunity()}>
        <CreateChannelModal
          isOpen={showCreateChannel()}
          communityId={selectedCommunityId()}
          onClose={() => setShowCreateChannel(false)}
        />
      </Show>
    </div>
  );
};

export default CommunityWindow;
