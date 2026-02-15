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
import { commands } from "../ipc/commands";
import { communityState, setCommunityState } from "../stores/community.store";
import { authState } from "../stores/auth.store";
import { voiceState, setVoiceState } from "../stores/voice.store";
import { subscribeChatEvents, subscribeCommunityEvents, subscribeVoiceEvents, subscribePresenceEvents } from "../ipc/channels";
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
import type { ChatEvent, CommunityEvent, VoiceEvent, PresenceEvent } from "../ipc/channels";
import type { Message } from "../stores/chat.store";
import {
  ICON_COMMUNITIES,
  ICON_PLUS,
  ICON_PLUS_BOX,
  ICON_SETTINGS,
  ICON_LOGOUT,
  ICON_CHANNEL_TEXT,
} from "../icons";

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
        const memberIdx = community.members.findIndex((m) => m.pseudonymKey === key);
        if (memberIdx >= 0) {
          setCommunityState("communities", communityId, "members", memberIdx, "status", newStatus);
        }
      }
    }));

    // Subscribe to community events for member roster changes
    unlisteners.push(subscribeCommunityEvents((event: CommunityEvent) => {
      if (event.type === "memberJoined") {
        const { communityId, pseudonymKey, displayName, roleIds } = event.data;
        const community = communityState.communities[communityId];
        if (community) {
          const exists = community.members.some((m) => m.pseudonymKey === pseudonymKey);
          if (!exists) {
            setCommunityState("communities", communityId, "members", (prev) => [
              ...prev,
              { pseudonymKey, displayName, roleIds, displayRole: "", status: "online", timeoutUntil: null },
            ]);
          }
        }
      } else if (event.type === "memberRemoved") {
        const { communityId, pseudonymKey } = event.data;
        setCommunityState("communities", communityId, "members", (prev) =>
          prev.filter((m) => m.pseudonymKey !== pseudonymKey),
        );
      } else if (event.type === "rolesChanged") {
        const { communityId, roles } = event.data;
        if (communityState.communities[communityId]) {
          setCommunityState("communities", communityId, "roles", roles);
        }
      } else if (event.type === "memberRolesChanged") {
        const { communityId, pseudonymKey, roleIds: newRoleIds } = event.data;
        const community = communityState.communities[communityId];
        if (community) {
          const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
          if (idx >= 0) {
            setCommunityState("communities", communityId, "members", idx, "roleIds", newRoleIds);
          }
          // If it's us, update myRoleIds
          if (pseudonymKey === community.myPseudonymKey) {
            setCommunityState("communities", communityId, "myRoleIds", newRoleIds);
          }
        }
      } else if (event.type === "memberTimedOut") {
        const { communityId, pseudonymKey, timeoutUntil } = event.data;
        const community = communityState.communities[communityId];
        if (community) {
          const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
          if (idx >= 0) {
            setCommunityState("communities", communityId, "members", idx, "timeoutUntil", timeoutUntil);
          }
        }
      } else if (event.type === "channelOverwriteChanged") {
        // Channel overwrites changed — re-fetch members/details to update permission state
        const { communityId } = event.data;
        if (communityState.communities[communityId]) {
          commands.getCommunityDetails().then((details) => {
            const detail = details.find((d: { id: string }) => d.id === communityId);
            if (detail) {
              setCommunityState("communities", communityId, "roles", detail.roles);
            }
          }).catch(() => {});
        }
      } else if (event.type === "mekRotated") {
        const { communityId, newGeneration } = event.data;
        if (communityState.communities[communityId]) {
          setCommunityState("communities", communityId, "mekGeneration", newGeneration);
        }
      } else if (event.type === "kicked") {
        const { communityId } = event.data;
        setCommunityState("communities", communityId, undefined!);
        if (communityState.activeCommunity === communityId) {
          setCommunityState("activeCommunity", null);
          setCommunityState("activeChannel", null);
        }
      }
    }));

    // Subscribe to chat events for incoming channel messages from other users.
    // Our own messages are handled via optimistic insert in handleSendChannelMessage.
    unlisteners.push(subscribeChatEvents((event: ChatEvent) => {
      if (event.type === "MessageReceived") {
        // Skip our own messages — already added optimistically
        const myPseudo = activeCommunity()?.myPseudonymKey;
        if (myPseudo && event.data.from === myPseudo) return;
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
      } else if (event.type === "ChannelHistoryLoaded") {
        const { channelId, messages: serverMsgs } = event.data;
        const existing = communityState.channelMessages[channelId] ?? [];
        // Deduplicate by timestamp + senderId
        const existingKeys = new Set(
          existing.map((m) => `${m.timestamp}:${m.senderId}`),
        );
        const newMsgs: Message[] = serverMsgs
          .filter((m) => !existingKeys.has(`${m.timestamp}:${m.senderId}`))
          .map((m) => ({
            id: m.id,
            senderId: m.senderId,
            body: m.body,
            timestamp: m.timestamp,
            isOwn: m.isOwn,
          }));
        if (newMsgs.length > 0) {
          const merged = [...existing, ...newMsgs].sort(
            (a, b) => a.timestamp - b.timestamp,
          );
          setCommunityState("channelMessages", channelId, merged);
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
