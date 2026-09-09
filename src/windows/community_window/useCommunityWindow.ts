import { createEffect, onMount, onCleanup } from "solid-js";
import { type UnlistenFn } from "@tauri-apps/api/event";
import { communityState, setCommunityState } from "../../stores/community.store";
import { authState } from "../../stores/auth.store";
import { initVoiceEventListener } from "../../actions/voice.actions";
import { subscribeCommunityChannelChatEvents } from "../../handlers/chat-events.handlers";
import { subscribeCommunityPresenceEvents } from "../../handlers/presence-events.handlers";
import {
  subscribeCommunityEventDispatcher,
  subscribeCommunityVoiceEvents,
} from "../../handlers/community.handlers";
import { hydrateState } from "../../stores/hydrate";
import {
  handleLoadChannelMessages,
  handleLoadOlderMessages,
  handleSendChannelMessage,
  handleSelectCommunity as storeSyncSelectCommunity,
  handleSelectChannel as storeSyncSelectChannel,
  handleGetChannelPins,
  handleSendChannelTyping,
  handleLoadThreadMessages,
  handleArchiveThread,
  handleLoadChannelThreads,
  handleLoadGameServers,
  handleLoadEvents,
  handleLoadOnboardingConfig,
  handleLoadWelcomeScreen,
} from "../../actions/community.actions";
import { commands } from "../../ipc/commands";
import { ICON_CHANNEL_TEXT, ICON_MEGAPHONE } from "../../icons";
import type { Message } from "../../stores/chat.store";
import type { Thread } from "../../stores/community.store";
import { createCommunityWindowState } from "./state";

/// Orchestrating view-model hook for `CommunityWindow`. Owns the effects,
/// event handlers, and Veilid subscription lifecycle on top of the signals
/// and memos from `createCommunityWindowState`. Runs inside the component
/// owner context so `createEffect`/`onMount`/`onCleanup` are valid here.
export function useCommunityWindow() {
  const s = createCommunityWindowState();
  const {
    getCommunityFromUrl, selectedCommunityId, setSelectedCommunityId,
    selectedChannelId, setSelectedChannelId, activeCommunity, activeChannel,
    channelMessages, gameServers, gameNameCache, setGameNameCache,
    setShowWelcomeForCommunity, setActiveThread, setShowEvents, setShowServers,
    setRightPanel, rightPanel, setReplyTo, setEditState, setHasMoreOlder, setPins,
    hasMoreOlder, isLoadingOlder, setIsLoadingOlder, createThreadTarget,
    setCreateThreadTarget, setSearchInitialChannel, setShowSearch,
  } = s;

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
    if (s.activeThread()?.id === threadId) {
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
    const { handleCreateThread } = await import("../../actions/community.actions");
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
    unlisteners.push(subscribeCommunityVoiceEvents());
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

  return {
    ...s,
    handleTyping, handleSelectCommunity, handleSelectChannel, handleTogglePins,
    handleToggleThreadList, handleToggleMembers, handleOpenThread, handleCloseThread,
    handleArchiveActiveThread, openCreateThreadModal, handleSubmitCreateThread,
    handleLoadOlder, handleReply, handleChannelSend, channelHeaderIcon, handleJumpToMessage,
  };
}

export type CommunityVm = ReturnType<typeof useCommunityWindow>;
