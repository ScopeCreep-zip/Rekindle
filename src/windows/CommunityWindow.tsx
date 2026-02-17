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
import CommunitySettingsModal from "../components/community/CommunitySettingsModal";
import RenameChannelModal from "../components/community/RenameChannelModal";
import ConfirmDialog from "../components/common/ConfirmDialog";
import ToastContainer from "../components/common/Toast";
import { communityState } from "../stores/community.store";
import { authState } from "../stores/auth.store";
import { voiceState } from "../stores/voice.store";
import { initVoiceEventListener } from "../handlers/voice.handlers";
import { subscribeCommunityChannelChatEvents } from "../handlers/chat-events.handlers";
import { subscribeCommunityPresenceEvents } from "../handlers/presence-events.handlers";
import { subscribeCommunityEventDispatcher } from "../handlers/community.handlers";
import { hydrateState } from "../ipc/hydrate";
import {
  handleLoadChannelMessages,
  handleSendChannelMessage,
  handleSelectCommunity as storeSyncSelectCommunity,
  handleSelectChannel as storeSyncSelectChannel,
  handleLeaveCommunity,
  handleDeleteChannel,
  handleRetryChannelMessage,
} from "../handlers/community.handlers";
import { handleJoinVoice } from "../handlers/voice.handlers";
import {
  calculateBasePermissions,
  hasPermission,
  MANAGE_CHANNELS,
} from "../ipc/permissions";
import type { Message } from "../stores/chat.store";
import {
  ICON_COMMUNITIES,
  ICON_PLUS,
  ICON_PLUS_BOX,
  ICON_SETTINGS,
  ICON_LOGOUT,
  ICON_CHANNEL_TEXT,
} from "../icons";

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
  const [showSettings, setShowSettings] = createSignal(false);
  const [renameTarget, setRenameTarget] = createSignal<{ channelId: string; currentName: string } | null>(null);
  const [showLeaveConfirm, setShowLeaveConfirm] = createSignal(false);

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

  const myRoleIds = createMemo((): number[] => {
    const community = activeCommunity();
    return community?.myRoleIds ?? [];
  });

  const canManageChannels = createMemo((): boolean => {
    const community = activeCommunity();
    if (!community) return false;
    const perms = calculateBasePermissions(myRoleIds(), community.roles, community.isHosted);
    return hasPermission(perms, MANAGE_CHANNELS);
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

    unlisteners.push(initVoiceEventListener());
    unlisteners.push(subscribeCommunityPresenceEvents());
    unlisteners.push(subscribeCommunityEventDispatcher());
    unlisteners.push(subscribeCommunityChannelChatEvents(() => activeCommunity()?.myPseudonymKey));
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
                <span class="nf-icon">{ICON_COMMUNITIES}</span>
              </button>
              <button
                class="action-bar-btn header-add-btn"
                onClick={() => setShowCreateCommunity(true)}
                title="Create Community"
              >
                <span class="nf-icon">{ICON_PLUS}</span>
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
              <span class="header-btn-group">
                <button
                  class="action-bar-btn header-add-btn"
                  onClick={() => setShowSettings(true)}
                  title="Community Settings"
                >
                  <span class="nf-icon">{ICON_SETTINGS}</span>
                </button>
                <Show when={canManageChannels()}>
                  <button
                    class="action-bar-btn header-add-btn"
                    onClick={() => setShowCreateChannel(true)}
                    title="Create Channel"
                  >
                    <span class="nf-icon">{ICON_PLUS_BOX}</span>
                  </button>
                </Show>
              </span>
            </div>
            <ChannelList
              channels={activeCommunity()!.channels}
              selectedId={selectedChannelId()}
              communityId={selectedCommunityId()}
              canManage={canManageChannels()}
              onSelect={handleSelectChannel}
              onVoiceJoin={handleJoinVoice}
              onRename={(channelId, currentName) => setRenameTarget({ channelId, currentName })}
              onDelete={(channelId) => handleDeleteChannel(selectedCommunityId(), channelId)}
            />
          </Show>
          <Show when={voiceState.isConnected}>
            <VoicePanel />
          </Show>
          <Show when={activeCommunity()}>
            <div class="community-header-actions">
              <button
                class="community-leave-btn"
                onClick={() => setShowLeaveConfirm(true)}
              >
                <span class="nf-icon">{ICON_LOGOUT}</span> Leave
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
              <span class="nf-icon community-channel-header-icon">{ICON_CHANNEL_TEXT}</span>
              {activeChannel()!.name}
              <Show when={activeCommunity()?.description}>
                <span class="community-description-hint">{activeCommunity()!.description}</span>
              </Show>
            </div>
            <MessageList
              messages={channelMessages()}
              ownName={authState.displayName ?? "You"}
              peerName="Channel"
              onRetry={(messageId) => handleRetryChannelMessage(selectedChannelId(), messageId)}
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
              myRoleIds={myRoleIds()}
              roles={activeCommunity()!.roles}
              myPseudonymKey={activeCommunity()?.myPseudonymKey ?? null}
              isHosted={activeCommunity()?.isHosted ?? false}
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
        <CommunitySettingsModal
          isOpen={showSettings()}
          community={activeCommunity()!}
          myRoleIds={myRoleIds()}
          onClose={() => setShowSettings(false)}
        />
      </Show>
      <Show when={renameTarget()}>
        {(target) => (
          <RenameChannelModal
            isOpen={true}
            communityId={selectedCommunityId()}
            channelId={target().channelId}
            currentName={target().currentName}
            onClose={() => setRenameTarget(null)}
          />
        )}
      </Show>
      <ConfirmDialog
        isOpen={showLeaveConfirm()}
        title="Leave Community"
        message={`Leave ${activeCommunity()?.name ?? "this community"}? You will need to be re-invited to rejoin.`}
        danger
        confirmLabel="Leave"
        onConfirm={() => {
          handleLeaveCommunity(selectedCommunityId());
          setShowLeaveConfirm(false);
        }}
        onCancel={() => setShowLeaveConfirm(false)}
      />
      <ToastContainer />
    </div>
  );
};

export default CommunityWindow;
