import { Component, createSignal, createMemo, createEffect, Show, Switch, Match, onMount, onCleanup } from "solid-js";
import { type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import CommunityList from "../components/community/CommunityList";
import ChannelList from "../components/community/ChannelList";
import MemberList from "../components/community/MemberList";
import MessageList from "../components/chat/MessageList";
import MessageInput from "../components/chat/MessageInput";
import ForwardMessageDialog from "../components/chat/ForwardMessageDialog";
import VoicePanel from "../components/voice/VoicePanel";
import CreateCommunityModal from "../components/community/CreateCommunityModal";
import CreateChannelModal from "../components/community/CreateChannelModal";
import JoinCommunityModal from "../components/community/JoinCommunityModal";
import CommunitySettingsModal from "../components/community/CommunitySettingsModal";
import RenameChannelModal from "../components/community/RenameChannelModal";
import RenameCategoryModal from "../components/community/RenameCategoryModal";
import CreateCategoryModal from "../components/community/CreateCategoryModal";
import CreateEventModal from "../components/community/CreateEventModal";
import CreatePollModal from "../components/community/CreatePollModal";
import EventsPanel from "../components/community/EventsPanel";
import GameServerList from "../components/community/GameServerList";
import OnboardingWizard from "../components/community/OnboardingWizard";
import ThreadPanel from "../components/community/ThreadPanel";
import ThreadListPanel from "../components/community/ThreadListPanel";
import ForumChannelView from "../components/community/ForumChannelView";
import SearchPanel from "../components/chat/SearchPanel";
import VideoCallPanel from "../components/voice/VideoCallPanel";
import CreateThreadModal from "../components/community/CreateThreadModal";
import StagePanel from "../components/community/StagePanel";
import PinnedMessagesPanel from "../components/community/PinnedMessagesPanel";
import CategoryHeader from "../components/community/CategoryHeader";
import WelcomeScreen from "../components/community/WelcomeScreen";
import ConfirmDialog from "../components/common/ConfirmDialog";
import SimpleInputModal from "../components/common/SimpleInputModal";
import ToastContainer from "../components/common/Toast";
import { communityState, setCommunityState } from "../stores/community.store";
import { authState } from "../stores/auth.store";
import { voiceState } from "../stores/voice.store";
import { initVoiceEventListener } from "../handlers/voice.handlers";
import { subscribeCommunityChannelChatEvents } from "../handlers/chat-events.handlers";
import { subscribeCommunityPresenceEvents } from "../handlers/presence-events.handlers";
import {
  subscribeCommunityEventDispatcher,
  typingUsers,
} from "../handlers/community.handlers";
import { hydrateState } from "../ipc/hydrate";
import {
  handleLoadChannelMessages,
  handleLoadOlderMessages,
  handleSendChannelMessage,
  handleSelectCommunity as storeSyncSelectCommunity,
  handleSelectChannel as storeSyncSelectChannel,
  handleLeaveCommunity,
  handleDeleteChannel,
  handleRetryChannelMessage,
  handleEditChannelMessage,
  handleDeleteChannelMessage,
  handleBulkDeleteChannelMessages,
  handleAddReaction,
  handleRemoveReaction,
  handlePinMessage,
  handleUnpinMessage,
  handleGetChannelPins,
  handleSendChannelTyping,
  handleSendThreadMessage,
  handleLoadThreadMessages,
  handleArchiveThread,
  handleVotePoll,
  handleClosePoll,
  handleLoadChannelThreads,
  handleCreateForumPost,
  handleSetChannelTopic,
  handleSetNotificationOverride,
  handleDeleteCategory,
  handleLoadGameServers,
  handleAddGameServer,
  handleRemoveGameServer,
  handleLoadEvents,
  handleLoadOnboardingConfig,
  handleLoadWelcomeScreen,
  handleSubmitOnboarding,
} from "../handlers/community.handlers";
import { handleJoinVoice, handleLeaveVoice, handleRequestToSpeak, handleRespondToSpeakRequest } from "../handlers/voice.handlers";
import {
  calculateBasePermissions,
  hasPermission,
  MANAGE_CHANNELS,
  MANAGE_MESSAGES,
  MANAGE_COMMUNITY,
  SEND_MESSAGES,
  BYPASS_SLOWMODE,
  REQUEST_TO_SPEAK,
} from "../ipc/permissions";
import { commands } from "../ipc/commands";
import { formatRelativeTime } from "../utils/formatting";
import type { Message } from "../stores/chat.store";
import type { EditMode } from "../components/chat/MessageInput";
import type { Thread, CommunityEvent } from "../stores/community.store";
import {
  ICON_COMMUNITIES,
  ICON_PLUS,
  ICON_PLUS_BOX,
  ICON_SETTINGS,
  ICON_LOGOUT,
  ICON_SEARCH,
  ICON_CHANNEL_TEXT,
  ICON_MEGAPHONE,
  ICON_PIN,
  ICON_THREAD,
  ICON_CALENDAR,
  ICON_SERVER,
} from "../icons";

type RightPanel = "members" | "pins" | "threadList" | "thread" | null;

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
  const [showCreateCategory, setShowCreateCategory] = createSignal(false);
  const [showCreateEvent, setShowCreateEvent] = createSignal(false);
  const [showEvents, setShowEvents] = createSignal(false);
  const [showServers, setShowServers] = createSignal(false);
  const [renameTarget, setRenameTarget] = createSignal<{ channelId: string; currentName: string } | null>(null);
  const [renameCategoryTarget, setRenameCategoryTarget] = createSignal<{ categoryId: string; currentName: string } | null>(null);
  const [showLeaveConfirm, setShowLeaveConfirm] = createSignal(false);
  // Architecture §23 — Cmd/Ctrl-F overlay for FTS5 message search.
  const [showSearch, setShowSearch] = createSignal(false);
  // Plan §Failure 9 — when the user opens search via Cmd/Ctrl-F we
  // seed the panel scope to the active channel; when they click the
  // titlebar Search button we seed to the entire community so the
  // SearchPanel's initial scope inference picks "community".
  const [searchInitialChannel, setSearchInitialChannel] = createSignal<
    string | null
  >(null);
  const [replyTo, setReplyTo] = createSignal<{ senderName: string; body: string; messageId?: string } | null>(null);
  const [pins, setPins] = createSignal<{ messageId: string; channelId: string; pinnedBy: string; pinnedAt: number }[]>([]);
  const [activeThread, setActiveThread] = createSignal<Thread | null>(null);
  const [editingTopic, setEditingTopic] = createSignal(false);
  const [topicDraft, setTopicDraft] = createSignal("");
  const [editState, setEditState] = createSignal<EditMode | null>(null);
  const [deleteTarget, setDeleteTarget] = createSignal<string | null>(null);
  const [isLoadingOlder, setIsLoadingOlder] = createSignal(false);
  const [hasMoreOlder, setHasMoreOlder] = createSignal(true);
  const [editingEvent, setEditingEvent] = createSignal<CommunityEvent | null>(null);
  const [createPollTarget, setCreatePollTarget] = createSignal<string | null>(null);
  const [createThreadTarget, setCreateThreadTarget] = createSignal<{ starterMessageId: string; initialName: string } | null>(null);
  const [forwardTarget, setForwardTarget] = createSignal<string | null>(null);
  const [showWelcomeForCommunity, setShowWelcomeForCommunity] = createSignal<string | null>(null);

  // Phase 3: Unified right panel state
  const [rightPanel, setRightPanel] = createSignal<RightPanel>("members");

  // Phase 2.2 & 2.3: Sidebar collapsible sections
  const [sidebarServersExpanded, setSidebarServersExpanded] = createSignal(true);
  const [sidebarEventsExpanded, setSidebarEventsExpanded] = createSignal(true);

  // Game name cache for sidebar server section
  const [gameNameCache, setGameNameCache] = createSignal<Map<string, string>>(new Map());

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

  const threadMessages = createMemo((): Message[] => {
    const thread = activeThread();
    if (!thread) return [];
    return communityState.threadMessages[thread.id] ?? [];
  });

  const memberNames = createMemo((): Record<string, string> => {
    const community = activeCommunity();
    if (!community) return {};
    const map: Record<string, string> = {};
    for (const m of community.members) {
      map[m.pseudonymKey] = m.displayName;
    }
    return map;
  });

  const myRoleIds = createMemo((): number[] => {
    const community = activeCommunity();
    return community?.myRoleIds ?? [];
  });

  const myPerms = createMemo((): bigint => {
    const community = activeCommunity();
    if (!community) return 0n;
    return calculateBasePermissions(myRoleIds(), community.roles);
  });

  const canManageChannels = createMemo(() => hasPermission(myPerms(), MANAGE_CHANNELS));
  const canManageMessages = createMemo(() => hasPermission(myPerms(), MANAGE_MESSAGES));
  // Architecture §10.7 — stage hand-raise. The audience-side button
  // disappears for members lacking REQUEST_TO_SPEAK; backend
  // re-validates on the SpeakRequest envelope.
  const canRequestToSpeak = createMemo(() => hasPermission(myPerms(), REQUEST_TO_SPEAK));
  const canManageCommunity = createMemo(() => hasPermission(myPerms(), MANAGE_COMMUNITY));
  const canSendMessages = createMemo(() => hasPermission(myPerms(), SEND_MESSAGES));
  const canBypassSlowmode = createMemo(() => hasPermission(myPerms(), BYPASS_SLOWMODE));

  const gameServers = createMemo(() => {
    const id = selectedCommunityId();
    return id ? (communityState.gameServers[id] ?? []) : [];
  });

  const isAnnouncementChannel = createMemo(() => activeChannel()?.type === "announcement");
  const isForumChannel = createMemo(() => activeChannel()?.type === "forum");
  const isStageChannel = createMemo(() => activeChannel()?.type === "stage");
  const isVoiceChannel = createMemo(() => activeChannel()?.type === "voice");
  /// Architecture §10.8 text-in-voice: in a voice channel, the chat
  /// pane is visible only while the local member is connected to that
  /// channel's voice session. For non-voice channels this is `true` by
  /// definition so the chat pane always renders.
  const isTextPaneVisible = createMemo(() => {
    if (!isVoiceChannel()) return true;
    return voiceState.isConnected && voiceState.channelId === selectedChannelId();
  });
  const canPostInChannel = createMemo(() => {
    if (!isAnnouncementChannel()) return canSendMessages();
    return canManageCommunity();
  });
  const activeVoiceChannelState = createMemo(() => {
    const channelId = selectedChannelId();
    return channelId ? communityState.voiceChannels[channelId] : undefined;
  });
  const isConnectedToActiveStage = createMemo(() =>
    voiceState.isConnected
    && voiceState.channelId === selectedChannelId(),
  );

  const channelTypingUsers = createMemo(() => {
    const channelId = selectedChannelId();
    if (!channelId) return [];
    return typingUsers[channelId] ?? [];
  });

  // Upcoming events for sidebar (max 2)
  const upcomingEvents = createMemo((): CommunityEvent[] => {
    const community = activeCommunity();
    if (!community?.events) return [];
    const now = Math.floor(Date.now() / 1000);
    return community.events
      .filter((e) => e.status === "scheduled" && e.startTime > now)
      .sort((a, b) => a.startTime - b.startTime)
      .slice(0, 2);
  });

  const shouldShowOnboarding = createMemo(() => {
    const community = activeCommunity();
    return Boolean(community?.onboardingConfig?.enabled && community.onboardingComplete === false);
  });

  const shouldShowWelcome = createMemo(() => {
    const community = activeCommunity();
    return Boolean(
      community
      && showWelcomeForCommunity() === community.id
      && community.welcomeScreen,
    );
  });

  // Sidebar servers (max 3)
  const sidebarServerList = createMemo(() => gameServers().slice(0, 3));

  // Resolve game names for sidebar servers
  createEffect(() => {
    const servers = gameServers();
    const cache = gameNameCache();
    for (const s of servers) {
      if (s.gameId.match(/^\d+$/) && !cache.has(s.gameId)) {
        commands.getGameName(parseInt(s.gameId, 10)).then((name) => {
          if (name) {
            setGameNameCache((prev) => {
              const next = new Map(prev);
              next.set(s.gameId, name);
              return next;
            });
          }
        });
      }
    }
  });

  function formatTimeUntilEvent(timestamp: number): string {
    const now = Math.floor(Date.now() / 1000);
    const diff = timestamp - now;
    if (diff <= 0) return "Started";
    if (diff < 3600) return `In ${Math.floor(diff / 60)}m`;
    if (diff < 86400) return `In ${Math.floor(diff / 3600)}h`;
    return `In ${Math.floor(diff / 86400)}d`;
  }

  // Load messages when channel changes
  createEffect(() => {
    const channelId = selectedChannelId();
    if (channelId) {
      handleLoadChannelMessages(channelId, 50);
    }
  });

  createEffect(() => {
    const communityId = selectedCommunityId();
    const channel = activeChannel();
    if (communityId && channel?.type === "forum") {
      void handleLoadChannelThreads(communityId, channel.id);
    }
  });

  createEffect(() => {
    const community = activeCommunity();
    if (!community) return;
    if (community.onboardingComplete && community.welcomeScreen) {
      setShowWelcomeForCommunity(community.id);
    }
  });

  // Typing indicator debounce
  let typingTimeout: number | undefined;
  function handleTyping(): void {
    const communityId = selectedCommunityId();
    const channelId = selectedChannelId();
    if (!communityId || !channelId) return;
    if (typingTimeout) return;
    handleSendChannelTyping(communityId, channelId);
    typingTimeout = window.setTimeout(() => { typingTimeout = undefined; }, 3000);
  }

  function handleSelectCommunity(id: string) {
    setSelectedCommunityId(id);
    storeSyncSelectCommunity(id);
    const community = communityState.communities[id];
    if (community?.channels.length) {
      const firstText = community.channels.find((c) =>
        c.type === "text" || c.type === "announcement" || c.type === "forum");
      if (firstText) {
        setSelectedChannelId(firstText.id);
        storeSyncSelectChannel(firstText.id);
      }
    }
    setActiveThread(null);
    setShowEvents(false);
    setShowServers(false);
    setShowWelcomeForCommunity(null);
    setRightPanel("members");
    handleLoadGameServers(id);
    handleLoadEvents(id);
    void handleLoadOnboardingConfig(id);
    void handleLoadWelcomeScreen(id);
  }

  function handleSelectChannel(id: string) {
    setSelectedChannelId(id);
    storeSyncSelectChannel(id);
    setReplyTo(null);
    setEditState(null);
    setActiveThread(null);
    setShowWelcomeForCommunity(null);
    setRightPanel("members");
    setHasMoreOlder(true);
  }

  async function handleTogglePins(): Promise<void> {
    if (rightPanel() === "pins") {
      setRightPanel("members");
      return;
    }
    const communityId = selectedCommunityId();
    const channelId = selectedChannelId();
    if (!communityId || !channelId) return;
    const result = await handleGetChannelPins(communityId, channelId);
    setPins(result);
    setRightPanel("pins");
  }

  function handleToggleThreadList(): void {
    if (rightPanel() === "threadList") {
      setRightPanel("members");
      return;
    }
    const communityId = selectedCommunityId();
    const channelId = selectedChannelId();
    if (communityId && channelId) {
      handleLoadChannelThreads(communityId, channelId);
      setRightPanel("threadList");
    }
  }

  function handleToggleMembers(): void {
    // Toggle: clicking the members button when the members panel is
    // already active collapses the right rail entirely (`null`); from
    // any other panel it switches to members. Mirrors the toggle
    // semantics of `handleTogglePins` / `handleToggleThreadList`.
    setRightPanel(rightPanel() === "members" ? null : "members");
  }

  function handleOpenThread(thread: Thread): void {
    setActiveThread(thread);
    setRightPanel("thread");
    handleLoadThreadMessages(selectedCommunityId(), thread.id, 100);
  }

  function handleCloseThread(): void {
    setActiveThread(null);
    setCommunityState("activeThread", null);
    setRightPanel("members");
  }

  async function handleArchiveActiveThread(
    communityId: string,
    threadId: string,
  ): Promise<void> {
    await handleArchiveThread(communityId, threadId);
    if (activeThread()?.id === threadId) {
      handleCloseThread();
    }
  }

  function openCreateThreadModal(starterMessageId: string): void {
    setCreateThreadTarget({
      starterMessageId,
      initialName: "",
    });
  }

  async function handleSubmitCreateThread(name: string, autoArchiveSeconds: number): Promise<void> {
    const target = createThreadTarget();
    if (!target) return;
    const { handleCreateThread } = await import("../handlers/community.handlers");
    const threadId = await handleCreateThread(
      selectedCommunityId(),
      selectedChannelId(),
      name,
      target.starterMessageId,
      undefined,
      autoArchiveSeconds,
    );
    if (!threadId) {
      throw new Error("Failed to create thread");
    }
    const thread = (communityState.channelThreads[selectedChannelId()] ?? [])
      .find((item) => item.id === threadId);
    if (thread) {
      handleOpenThread(thread);
    }
    setCreateThreadTarget(null);
  }

  async function handleLoadOlder(): Promise<void> {
    const communityId = selectedCommunityId();
    const channelId = selectedChannelId();
    if (!communityId || !channelId || isLoadingOlder() || !hasMoreOlder()) return;

    const msgs = channelMessages();
    if (msgs.length === 0) return;
    const oldest = msgs[0];

    setIsLoadingOlder(true);
    try {
      const hasMore = await handleLoadOlderMessages(communityId, channelId, oldest.timestamp, 50);
      setHasMoreOlder(hasMore);
    } finally {
      setIsLoadingOlder(false);
    }
  }

  function handleReply(message: Message): void {
    setReplyTo({
      senderName: message.isOwn ? (authState.displayName ?? "You") : message.senderId,
      body: message.body,
      messageId: message.serverMessageId,
    });
  }

  function handleChannelSend(channelId: string, body: string, replyToId?: string): void {
    handleSendChannelMessage(channelId, body, replyToId);
  }

  function channelHeaderIcon(): string {
    const ch = activeChannel();
    if (ch?.type === "announcement") return ICON_MEGAPHONE;
    return ICON_CHANNEL_TEXT;
  }

  function handleJumpToMessage(messageId: string): void {
    const el = document.querySelector(`[data-message-id="${messageId}"]`);
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
      el.classList.add("message-highlight");
      setTimeout(() => el.classList.remove("message-highlight"), 1500);
    }
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

    // Architecture §23 — global Cmd/Ctrl-F opens the message search overlay.
    window.addEventListener("keydown", searchShortcutHandler);
  });

  function searchShortcutHandler(e: KeyboardEvent): void {
    // Architecture §23 — accept both Cmd/Ctrl-F (legacy chat-app
    // muscle memory) and Cmd/Ctrl-K (Slack/Linear-style command
    // palette key) so users land on the search overlay regardless of
    // which mental model they bring.
    const key = e.key.toLowerCase();
    const isShortcut = (e.metaKey || e.ctrlKey) && (key === "f" || key === "k");
    if (isShortcut) {
      e.preventDefault();
      setSearchInitialChannel(selectedChannelId() || null);
      setShowSearch(true);
    }
  }

  onCleanup(() => {
    for (const p of unlisteners) {
      p.then((unlisten) => unlisten());
    }
    window.removeEventListener("keydown", searchShortcutHandler);
  });

  return (
    <div class="app-frame">
      {/* Architecture §32 a11y — keyboard skip link. Hidden until
       * focused, then jumps past the navigation rails to the message
       * area. The MessageList container is given `id="main-content"`
       * + `tabindex="-1"` so the anchor focus lands inside it. */}
      <a href="#main-content" class="skip-link">Skip to messages</a>
      <Titlebar title={activeCommunity()?.name ?? "Community"} showMaximize />
      {/* Architecture §17.4 — raid alert banner. Rendered as a
       * top-of-window overlay (role="alert" so screen readers
       * announce immediately) whenever the backend's raid detector
       * has fired and is unresolved. */}
      {/* Plan §Failure 11 — banner stays until a moderator
       *  acknowledges. Dismiss only clears the local flag (the
       *  raid-detector keeps tripping if the join-flood continues
       *  next interval). */}
      <Show when={activeCommunity()?.raidAlertActive}>
        <div class="raid-alert-banner" role="alert">
          <span class="raid-alert-banner-text">
            <strong>Raid detected</strong> — slowmode and join-throttling are
            active. Moderators can review recent joins and pause invites.
          </span>
          <button
            type="button"
            class="form-btn-secondary"
            onClick={() => {
              const id = selectedCommunityId();
              if (id) {
                setCommunityState("communities", id, "raidAlertActive", false);
              }
            }}
          >
            Dismiss
          </button>
        </div>
      </Show>
      <div class="community-layout">
        {/* Left sidebar: community list + channel list + sidebar sections */}
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
            onSettings={(id) => { handleSelectCommunity(id); setShowSettings(true); }}
            onLeave={() => setShowLeaveConfirm(true)}
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
              categories={activeCommunity()!.categories}
              selectedId={selectedChannelId()}
              communityId={selectedCommunityId()}
              canManage={canManageChannels()}
              onSelect={handleSelectChannel}
              onVoiceJoin={(channelId) => handleJoinVoice(channelId, selectedCommunityId())}
              onRename={(channelId, currentName) => setRenameTarget({ channelId, currentName })}
              onDelete={(channelId) => handleDeleteChannel(selectedCommunityId(), channelId)}
              onRenameCategory={(categoryId, currentName) => setRenameCategoryTarget({ categoryId, currentName })}
              onDeleteCategory={(categoryId) => handleDeleteCategory(selectedCommunityId(), categoryId)}
              onCreateCategory={() => setShowCreateCategory(true)}
              onSetNotification={(channelId, level) => handleSetNotificationOverride(selectedCommunityId(), channelId, level)}
            />

            {/* Sidebar: Game Servers (collapsible) */}
            <Show when={gameServers().length > 0}>
              <div class="sidebar-servers-section">
                <CategoryHeader
                  name={`Servers (${gameServers().length})`}
                  isExpanded={sidebarServersExpanded()}
                  onToggle={() => setSidebarServersExpanded(!sidebarServersExpanded())}
                />
                <Show when={sidebarServersExpanded()}>
                  {sidebarServerList().map((server) => (
                    <div class="sidebar-server-row">
                      <span class="sidebar-server-name">
                        {gameNameCache().get(server.gameId) ?? server.label}
                      </span>
                      <button
                        class="sidebar-server-join-btn"
                        onClick={() => {
                          const id = parseInt(server.gameId, 10);
                          if (!isNaN(id)) commands.launchGameToServer(id, server.address);
                        }}
                      >
                        Join
                      </button>
                    </div>
                  ))}
                  <Show when={gameServers().length > 3}>
                    <button
                      class="sidebar-section-more"
                      onClick={() => { setShowServers(true); setShowEvents(false); }}
                    >
                      View all ({gameServers().length})
                    </button>
                  </Show>
                </Show>
              </div>
            </Show>

            {/* Sidebar: Upcoming Events (collapsible) */}
            <Show when={upcomingEvents().length > 0}>
              <div class="sidebar-events-section">
                <CategoryHeader
                  name="Upcoming"
                  isExpanded={sidebarEventsExpanded()}
                  onToggle={() => setSidebarEventsExpanded(!sidebarEventsExpanded())}
                />
                <Show when={sidebarEventsExpanded()}>
                  {upcomingEvents().map((event) => (
                    <div class="sidebar-event-row">
                      <span class="sidebar-event-title">{event.title}</span>
                      <span class="sidebar-event-countdown">{formatTimeUntilEvent(event.startTime)}</span>
                    </div>
                  ))}
                  <button
                    class="sidebar-section-more"
                    onClick={() => { setShowEvents(true); setShowServers(false); }}
                  >
                    All events
                  </button>
                </Show>
              </div>
            </Show>
          </Show>
          <Show when={voiceState.isConnected}>
            <VoicePanel />
          </Show>
          {/* Architecture §10.6 — video / screen-share UI overlay,
              active only while in a community voice channel. */}
          <Show
            when={
              voiceState.isConnected
              && voiceState.activeCallType === "community"
              && voiceState.channelId
              && selectedCommunityId()
            }
          >
            <VideoCallPanel
              communityId={selectedCommunityId()}
              channelId={voiceState.channelId!}
              visible
            />
          </Show>
          <Show when={activeCommunity()}>
            <div class="community-header-actions">
              {/* Plan §Failure 9 — global community search button. Seeds
               *  the panel with `channelId=null` so SearchPanel's scope
               *  inference picks "community" (search across every
               *  channel). Cmd/Ctrl-F still scopes to the active
               *  channel as before. */}
              <button
                class="community-leave-btn"
                title="Search this community (Cmd/Ctrl-Shift-F)"
                aria-label="Search community"
                onClick={() => {
                  setSearchInitialChannel(null);
                  setShowSearch(true);
                }}
              >
                <span class="nf-icon">{ICON_SEARCH}</span> Search
              </button>
              <button
                class="community-leave-btn"
                onClick={() => setShowLeaveConfirm(true)}
              >
                <span class="nf-icon">{ICON_LOGOUT}</span> Leave
              </button>
            </div>
          </Show>
        </div>

        {/* Main content: channel header + messages, events, or servers panel */}
        <div class="community-main" id="main-content" tabindex="-1">
          <Show when={showServers() && activeCommunity()}>
            <GameServerList
              servers={gameServers()}
              communityId={selectedCommunityId()}
              canManage={canManageCommunity()}
              onRemove={handleRemoveGameServer}
              onAdd={handleAddGameServer}
            />
          </Show>
          <Show when={!showServers()}>
          <Show when={showEvents() && activeCommunity()} fallback={
            <Show when={shouldShowWelcome() && activeCommunity()?.welcomeScreen} fallback={
              <Show when={activeChannel()} fallback={
                <div class="empty-placeholder">
                  <div class="empty-placeholder-title">Select a channel</div>
                  <div class="empty-placeholder-subtitle">Choose a community and channel to start chatting</div>
                </div>
              }>
                <div class="community-channel-header">
                  <span class="nf-icon community-channel-header-icon">{channelHeaderIcon()}</span>
                  {activeChannel()!.name}
                  <Show when={activeChannel()?.topic || canManageChannels()}>
                    <Show when={editingTopic()} fallback={
                      <span
                        class={`channel-topic ${canManageChannels() ? "channel-topic-editable" : ""}`}
                        onClick={() => {
                          if (canManageChannels()) {
                            setTopicDraft(activeChannel()?.topic ?? "");
                            setEditingTopic(true);
                          }
                        }}
                      >
                        {activeChannel()?.topic || (canManageChannels() ? "Set topic..." : "")}
                      </span>
                    }>
                      <input
                        class="form-input channel-topic-header-input"
                        type="text"
                        value={topicDraft()}
                        onInput={(e) => setTopicDraft(e.currentTarget.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            handleSetChannelTopic(selectedCommunityId(), selectedChannelId(), topicDraft());
                            setEditingTopic(false);
                          }
                          if (e.key === "Escape") {
                            setEditingTopic(false);
                          }
                        }}
                        onBlur={() => {
                          handleSetChannelTopic(selectedCommunityId(), selectedChannelId(), topicDraft());
                          setEditingTopic(false);
                        }}
                        placeholder="Channel topic..."
                      />
                    </Show>
                  </Show>
                  <Show when={activeCommunity()?.description && !activeChannel()?.topic && !editingTopic()}>
                    <span class="community-description-hint">{activeCommunity()!.description}</span>
                  </Show>
                  <span class="header-btn-group">
                    <button
                      class={`action-bar-btn header-add-btn ${rightPanel() === "pins" ? "header-btn-active" : ""}`}
                      onClick={handleTogglePins}
                      title="Pinned Messages"
                    >
                      <span class="nf-icon">{ICON_PIN}</span>
                      <Show when={pins().length > 0}>
                        <span class="channel-header-badge">{pins().length}</span>
                      </Show>
                    </button>
                    <button
                      class={`action-bar-btn header-add-btn ${rightPanel() === "threadList" || rightPanel() === "thread" ? "header-btn-active" : ""}`}
                      onClick={handleToggleThreadList}
                      title="Threads"
                    >
                      <span class="nf-icon">{ICON_THREAD}</span>
                      <Show when={(communityState.channelThreads[selectedChannelId()] ?? []).length > 0}>
                        <span class="channel-header-badge">{(communityState.channelThreads[selectedChannelId()] ?? []).length}</span>
                      </Show>
                    </button>
                    <button
                      class={`action-bar-btn header-add-btn ${rightPanel() === "members" ? "header-btn-active" : ""}`}
                      onClick={handleToggleMembers}
                      title="Members"
                      aria-label={rightPanel() === "members" ? "Hide members panel" : "Show members panel"}
                      aria-pressed={rightPanel() === "members"}
                    >
                      <span class="nf-icon" aria-hidden="true">{ICON_COMMUNITIES}</span>
                    </button>
                  </span>
                </div>
                <Show when={isForumChannel()} fallback={
                  <Show when={isStageChannel()} fallback={
                  <Show when={isTextPaneVisible()} fallback={
                    <div class="empty-placeholder voice-chat-locked">
                      <div class="empty-placeholder-title">Join voice to view chat</div>
                      <div class="empty-placeholder-subtitle">
                        Architecture §10.8 — text-in-voice messages are visible
                        only while you're connected to this voice channel.
                      </div>
                    </div>
                  }>
                  <>
                    <MessageList
                      communityId={selectedCommunityId()}
                      channelId={selectedChannelId()}
                      messages={channelMessages()}
                      ownName={authState.displayName ?? "You"}
                      peerName="Member"
                      memberNames={memberNames()}
                      myPseudonymKey={activeCommunity()?.myPseudonymKey}
                      threads={communityState.channelThreads[selectedChannelId()] ?? []}
                      canBulkDelete={canManageMessages()}
                      onBulkDelete={(messageIds) =>
                        handleBulkDeleteChannelMessages(
                          selectedCommunityId(),
                          selectedChannelId(),
                          messageIds,
                        )
                      }
                      onLoadOlder={handleLoadOlder}
                      isLoadingOlder={isLoadingOlder()}
                      onRetry={(messageId) => handleRetryChannelMessage(selectedChannelId(), messageId)}
                      onReply={handleReply}
                      onReaction={(messageId, emoji) => handleAddReaction(selectedCommunityId(), selectedChannelId(), messageId, emoji)}
                      onRemoveReaction={(messageId, emoji) => handleRemoveReaction(selectedCommunityId(), selectedChannelId(), messageId, emoji)}
                      onPin={(messageId) => {
                        const msg = channelMessages().find((m) => m.serverMessageId === messageId);
                        if (msg?.pinned) {
                          handleUnpinMessage(selectedCommunityId(), selectedChannelId(), messageId);
                        } else {
                          handlePinMessage(selectedCommunityId(), selectedChannelId(), messageId);
                        }
                      }}
                      onCreateThread={(messageId) => {
                        openCreateThreadModal(messageId);
                      }}
                      onCreatePoll={(messageId) => setCreatePollTarget(messageId)}
                      onOpenThread={handleOpenThread}
                      onEdit={(messageId, currentBody) => {
                        setEditState({ messageId, body: currentBody });
                      }}
                      onDelete={(messageId) => {
                        setDeleteTarget(messageId);
                      }}
                      onVotePoll={(pollId, selectedAnswers) =>
                        handleVotePoll(selectedCommunityId(), selectedChannelId(), pollId, selectedAnswers)
                      }
                      onClosePoll={(pollId) =>
                        handleClosePoll(selectedCommunityId(), selectedChannelId(), pollId)
                      }
                      onForward={(messageId) => setForwardTarget(messageId)}
                    />
                    <Show when={channelTypingUsers().length > 0}>
                      <div class="typing-indicator">
                        <span class="typing-dots">
                          <span class="typing-label">
                            {channelTypingUsers().map((u) => u.displayName).join(", ")} {channelTypingUsers().length === 1 ? "is" : "are"} typing...
                          </span>
                        </span>
                      </div>
                    </Show>
                    <MessageInput
                      communityId={selectedCommunityId()}
                      peerId={selectedChannelId()}
                      replyTo={replyTo()}
                      editMode={editState()}
                      onSend={handleChannelSend}
                      onTyping={handleTyping}
                      onDismissReply={() => setReplyTo(null)}
                      onEditSave={(messageId, newBody) => {
                        handleEditChannelMessage(selectedChannelId(), messageId, newBody);
                        setEditState(null);
                      }}
                      onEditCancel={() => setEditState(null)}
                      disabled={!canPostInChannel()}
                      disabledMessage={isAnnouncementChannel() ? "Only admins can post in announcement channels" : "You don't have permission to send messages"}
                      slowmodeSeconds={activeChannel()?.slowmodeSeconds}
                      bypassSlowmode={canBypassSlowmode()}
                    />
                  </>
                  </Show>
                  }>
                    <StagePanel
                      channel={activeChannel()!}
                      voiceChannel={activeVoiceChannelState()}
                      members={activeCommunity()!.members}
                      myPseudonymKey={activeCommunity()?.myPseudonymKey ?? null}
                      isConnectedToChannel={isConnectedToActiveStage()}
                      canModerate={canManageMessages()}
                      canRequestToSpeak={canRequestToSpeak()}
                      onJoinStage={() => void handleJoinVoice(selectedChannelId(), selectedCommunityId())}
                      onLeaveStage={() => void handleLeaveVoice()}
                      onRequestToSpeak={() => void handleRequestToSpeak(selectedCommunityId(), selectedChannelId())}
                      onApproveRequest={(requesterPseudonym) =>
                        void handleRespondToSpeakRequest(
                          selectedCommunityId(),
                          selectedChannelId(),
                          requesterPseudonym,
                          true,
                        )}
                      onDenyRequest={(requesterPseudonym) =>
                        void handleRespondToSpeakRequest(
                          selectedCommunityId(),
                          selectedChannelId(),
                          requesterPseudonym,
                          false,
                        )}
                    />
                  </Show>
                }>
                  <ForumChannelView
                    channel={activeChannel()!}
                    threads={communityState.channelThreads[selectedChannelId()] ?? []}
                    onOpenThread={handleOpenThread}
                    onCreatePost={async (name, body, forumTag) => {
                      const threadId = await handleCreateForumPost(
                        selectedCommunityId(),
                        selectedChannelId(),
                        name,
                        body,
                        forumTag,
                      );
                      if (!threadId) return;
                      const thread = (communityState.channelThreads[selectedChannelId()] ?? [])
                        .find((item) => item.id === threadId);
                      if (thread) handleOpenThread(thread);
                    }}
                  />
                </Show>
              </Show>
            }>
              <WelcomeScreen
                screen={activeCommunity()!.welcomeScreen!}
                communityName={activeCommunity()!.name}
                onChannelClick={(channelId) => {
                  handleSelectChannel(channelId);
                  setShowWelcomeForCommunity(null);
                }}
              />
            </Show>
          }>
            <EventsPanel
              communityId={selectedCommunityId()}
              myPseudonymKey={activeCommunity()?.myPseudonymKey ?? null}
              onCreateEvent={() => setShowCreateEvent(true)}
              onEditEvent={(event) => { setEditingEvent(event); setShowCreateEvent(true); }}
            />
          </Show>
          </Show>
        </div>

        {/* Phase 3: Unified right panel */}
        <Show when={activeCommunity()}>
          <div class="right-panel">
            <Switch>
              <Match when={rightPanel() === "members"}>
                <MemberList
                  members={activeCommunity()!.members}
                  communityId={selectedCommunityId()}
                  myRoleIds={myRoleIds()}
                  roles={activeCommunity()!.roles}
                  myPseudonymKey={activeCommunity()?.myPseudonymKey ?? null}
                />
              </Match>
              <Match when={rightPanel() === "pins"}>
                <PinnedMessagesPanel
                  pins={pins()}
                  messages={channelMessages()}
                  onClose={() => setRightPanel("members")}
                  onUnpin={(messageId) => {
                    handleUnpinMessage(selectedCommunityId(), selectedChannelId(), messageId);
                    setPins((prev) => prev.filter((p) => p.messageId !== messageId));
                  }}
                  onJumpToMessage={handleJumpToMessage}
                />
              </Match>
              <Match when={rightPanel() === "threadList"}>
                <ThreadListPanel
                  threads={communityState.channelThreads[selectedChannelId()] ?? []}
                  onSelectThread={(threadId) => {
                    const threads = communityState.channelThreads[selectedChannelId()] ?? [];
                    const thread = threads.find((t) => t.id === threadId);
                    if (thread) handleOpenThread(thread);
                  }}
                  onClose={() => setRightPanel("members")}
                />
              </Match>
              <Match when={rightPanel() === "thread"}>
                <ThreadPanel
                  thread={activeThread()}
                  communityId={selectedCommunityId()}
                  messages={threadMessages()}
                  onClose={handleCloseThread}
                  onSend={handleSendThreadMessage}
                  onArchive={handleArchiveActiveThread}
                  onReply={handleReply}
                  onReaction={(messageId, emoji) => handleAddReaction(selectedCommunityId(), selectedChannelId(), messageId, emoji)}
                  onRemoveReaction={(messageId, emoji) => handleRemoveReaction(selectedCommunityId(), selectedChannelId(), messageId, emoji)}
                  onPin={(messageId) => {
                    const msg = channelMessages().find((m) => m.serverMessageId === messageId);
                    if (msg?.pinned) {
                      handleUnpinMessage(selectedCommunityId(), selectedChannelId(), messageId);
                    } else {
                      handlePinMessage(selectedCommunityId(), selectedChannelId(), messageId);
                    }
                  }}
                  onCreatePoll={(messageId) => setCreatePollTarget(messageId)}
                  onEdit={(messageId, currentBody) => setEditState({ messageId, body: currentBody })}
                  onDelete={(messageId) => setDeleteTarget(messageId)}
                  onVotePoll={(pollId, selectedAnswers) =>
                    handleVotePoll(selectedCommunityId(), selectedChannelId(), pollId, selectedAnswers)
                  }
                  onClosePoll={(pollId) =>
                    handleClosePoll(selectedCommunityId(), selectedChannelId(), pollId)
                  }
                  myPseudonymKey={activeCommunity()?.myPseudonymKey}
                />
              </Match>
            </Switch>
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
        <Show when={showSearch()}>
          <SearchPanel
            communityId={selectedCommunityId()}
            channelId={searchInitialChannel()}
            onClose={() => setShowSearch(false)}
          />
        </Show>
        <CreateCategoryModal
          isOpen={showCreateCategory()}
          communityId={selectedCommunityId()}
          onClose={() => setShowCreateCategory(false)}
        />
        <CreateEventModal
          isOpen={showCreateEvent()}
          communityId={selectedCommunityId()}
          onClose={() => { setShowCreateEvent(false); setEditingEvent(null); }}
          isEditing={editingEvent() !== null}
          eventId={editingEvent()?.id}
          initialTitle={editingEvent()?.title}
          initialDescription={editingEvent()?.description}
          initialStartTime={editingEvent()?.startTime}
          initialEndTime={editingEvent()?.endTime ?? undefined}
          initialMaxAttendees={editingEvent()?.maxAttendees ?? undefined}
        />
        <CreatePollModal
          isOpen={createPollTarget() !== null}
          communityId={selectedCommunityId()}
          channelId={selectedChannelId()}
          messageId={createPollTarget() ?? ""}
          onClose={() => setCreatePollTarget(null)}
        />
        <CreateThreadModal
          isOpen={createThreadTarget() !== null}
          initialName={createThreadTarget()?.initialName ?? ""}
          onClose={() => setCreateThreadTarget(null)}
          onSubmit={async (name, autoArchive) => {
            try {
              await handleSubmitCreateThread(name, autoArchive);
              setCreateThreadTarget(null);
            } catch (e) {
              console.error("Failed to create thread:", e);
            }
          }}
        />
        <Show when={forwardTarget()}>
          {(messageId) => (
            <ForwardMessageDialog
              sourceCommunityId={selectedCommunityId()}
              sourceChannelId={selectedChannelId()}
              sourceMessageId={messageId()}
              onClose={() => setForwardTarget(null)}
            />
          )}
        </Show>
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
      <Show when={renameCategoryTarget()}>
        {(target) => (
          <RenameCategoryModal
            isOpen={true}
            communityId={selectedCommunityId()}
            categoryId={target().categoryId}
            currentName={target().currentName}
            onClose={() => setRenameCategoryTarget(null)}
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
      <ConfirmDialog
        isOpen={deleteTarget() !== null}
        title="Delete Message"
        message="Are you sure you want to delete this message? This cannot be undone."
        danger
        confirmLabel="Delete"
        onConfirm={() => {
          const msgId = deleteTarget();
          if (msgId) {
            handleDeleteChannelMessage(selectedChannelId(), msgId);
          }
          setDeleteTarget(null);
        }}
        onCancel={() => setDeleteTarget(null)}
      />
      <Show when={shouldShowOnboarding() && activeCommunity()?.onboardingConfig}>
        <OnboardingWizard
          communityId={selectedCommunityId()}
          config={activeCommunity()!.onboardingConfig!}
          onComplete={() => {
            setCommunityState("communities", selectedCommunityId(), "onboardingComplete", true);
            if (activeCommunity()?.welcomeScreen) {
              setShowWelcomeForCommunity(selectedCommunityId());
            }
          }}
          onCancel={
            activeCommunity()!.onboardingConfig!.mode === "gated"
              ? undefined
              : () => {
                  void handleSubmitOnboarding(selectedCommunityId(), []);
                }
          }
        />
      </Show>
      <ToastContainer />
    </div>
  );
};

export default CommunityWindow;
