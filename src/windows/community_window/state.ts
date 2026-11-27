import { createMemo, createSignal } from "solid-js";
import { typingUsers } from "../../actions/community.actions";
import type { EditMode } from "../../components/chat/message_input/types";
import type { CommunityCapability } from "../../ipc/commands/dto_capabilities";
import {
  BYPASS_SLOWMODE,
  hasPermission,
  MANAGE_CHANNELS,
  MANAGE_COMMUNITY,
  MANAGE_MESSAGES,
  REQUEST_TO_SPEAK,
  SEND_MESSAGES,
  servedPermissions,
} from "../../ipc/permissions";
import type { Message } from "../../stores/chat.store";
import type { ScheduledEvent, Thread } from "../../stores/community.store";
import { communityState } from "../../stores/community.store";
import { voiceState } from "../../stores/voice.store";
import { deniedFor } from "../../utils/denied";
import { formatEventCountdown as formatTimeUntilEvent } from "../../utils/time";

export type RightPanel = "members" | "pins" | "threadList" | "thread" | null;

function getCommunityFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("id") ?? "";
}

/** The channel the window was opened on (`&channel=`, plan E7.2 A), if any. */
function getChannelFromUrl(): string {
  return new URLSearchParams(window.location.search).get("channel") ?? "";
}

/// Signals + derived memos backing `CommunityWindow`. Split out of the
/// orchestrating `useCommunityWindow` hook so neither file approaches the
/// module size cap. Returns a flat object of stable accessor/setter
/// functions — destructuring it in callers is reactivity-safe because the
/// reactivity lives in the accessor functions, not the object reference.
export function createCommunityWindowState() {
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
  const [renameTarget, setRenameTarget] = createSignal<{
    channelId: string;
    currentName: string;
  } | null>(null);
  const [renameCategoryTarget, setRenameCategoryTarget] = createSignal<{
    categoryId: string;
    currentName: string;
  } | null>(null);
  // Architecture §23 — Cmd/Ctrl-F overlay for FTS5 message search.
  const [showSearch, setShowSearch] = createSignal(false);
  // Plan §Failure 9 — when the user opens search via Cmd/Ctrl-F we
  // seed the panel scope to the active channel; when they click the
  // titlebar Search button we seed to the entire community so the
  // SearchPanel's initial scope inference picks "community".
  const [searchInitialChannel, setSearchInitialChannel] = createSignal<string | null>(null);
  const [replyTo, setReplyTo] = createSignal<{
    senderName: string;
    body: string;
    messageId?: string;
  } | null>(null);
  const [pins, setPins] = createSignal<
    { messageId: string; channelId: string; pinnedBy: string; pinnedAt: number }[]
  >([]);
  // The open thread, as opened; `activeThread` below is only ever its live
  // row in the Host's thread list, so the header's count and activity follow
  // a reload (the opened copy kept "0 messages" after the card read
  // "1 message"). A thread the Host no longer lists is null, never the old
  // copy shown as live: the window closes its panel (useCommunityWindow).
  const [openedThread, setActiveThread] = createSignal<Thread | null>(null);
  const activeThread = createMemo((): Thread | null => {
    const opened = openedThread();
    if (!opened) return null;
    return (
      (communityState.channelThreads[opened.channelId] ?? []).find((t) => t.id === opened.id) ??
      null
    );
  });
  const [editingTopic, setEditingTopic] = createSignal(false);
  const [topicDraft, setTopicDraft] = createSignal("");
  const [editState, setEditState] = createSignal<EditMode | null>(null);
  const [isLoadingOlder, setIsLoadingOlder] = createSignal(false);
  const [hasMoreOlder, setHasMoreOlder] = createSignal(true);
  const [editingEvent, setEditingEvent] = createSignal<ScheduledEvent | null>(null);
  const [createPollTarget, setCreatePollTarget] = createSignal<string | null>(null);
  const [createThreadTarget, setCreateThreadTarget] = createSignal<{
    starterMessageId: string;
    initialName: string;
  } | null>(null);
  const [forwardTarget, setForwardTarget] = createSignal<string | null>(null);
  const [showWelcomeForCommunity, setShowWelcomeForCommunity] = createSignal<string | null>(null);

  // Voice call-stage: whether the §10.8 text-in-voice chat column is shown
  // alongside the participant gallery. Off by default so video is primary.
  const [showCallChat, setShowCallChat] = createSignal(false);

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

  /** The selected channel's threads not archived (the header badge). */
  const activeThreadCount = createMemo(
    () =>
      (communityState.channelThreads[selectedChannelId()] ?? []).filter((t) => !t.archived).length
  );

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

  /// A pseudonym's display name, as the channel message list shows it: our
  /// own key is our name in this community as the Host states it (never our
  /// identity name, plan E3.4 S6), a listed member's is the Host's, and an
  /// unlisted key is "Member" (never the raw 64-hex key).
  function memberName(pseudonymKey: string): string {
    if (pseudonymKey === activeCommunity()?.myPseudonymKey)
      return activeCommunity()?.myDisplayName ?? "Member";
    return memberNames()[pseudonymKey] ?? "Member";
  }

  /// What we may do here, as the backend computed it from the merged
  /// governance (plan E3.2 S9): the active channel's effective
  /// permissions (base plus its overwrites), else the base.
  const myPerms = createMemo((): bigint =>
    servedPermissions(activeCommunity(), activeChannel()?.id)
  );

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
  /// Our own removal as the backend reports it (plan E3.2 S9): the
  /// community stays as read-only history.
  const ownRemoval = createMemo(() => activeCommunity()?.removed ?? null);
  /// A join the inviter refused after it returned (plan JW): the Host's words.
  const joinWithdrawn = createMemo(() => activeCommunity()?.withdrawn?.reasonText ?? null);
  /// A restricted channel whose roster we are not on (FX3): the Host's words.
  const channelLocked = createMemo(() =>
    activeChannel()?.locked ? (activeChannel()?.lockedReason ?? "") : null
  );
  /// FX3: waiting for the community's current key from a member.
  /** The Host's labels for the governance writes it holds (S7d-keys.10). */
  const catchUpPending = createMemo(() => activeCommunity()?.catchUp?.pending ?? []);
  const keyPending = createMemo(() =>
    activeCommunity()?.keyStatus?.state === "pending" ? activeCommunity()!.keyStatus! : null
  );
  /// Why we may not do `cap` here, in the Host's words (never-hide), or null.
  const denied = (cap: CommunityCapability): string | null => deniedFor(activeCommunity(), cap);
  const canPostInChannel = createMemo(() => {
    if (ownRemoval() || joinWithdrawn()) return false;
    if (channelLocked() !== null) return false;
    if (!isAnnouncementChannel()) return canSendMessages();
    return canManageCommunity();
  });
  const activeVoiceChannelState = createMemo(() => {
    const channelId = selectedChannelId();
    return channelId ? communityState.voiceChannels[channelId] : undefined;
  });
  const isConnectedToActiveStage = createMemo(
    () => voiceState.isConnected && voiceState.channelId === selectedChannelId()
  );

  const channelTypingUsers = createMemo(() => {
    const channelId = selectedChannelId();
    if (!channelId) return [];
    return typingUsers[channelId] ?? [];
  });

  // Upcoming events for sidebar (max 2)
  const upcomingEvents = createMemo((): ScheduledEvent[] => {
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
      community && showWelcomeForCommunity() === community.id && community.welcomeScreen
    );
  });

  // Sidebar servers (max 3)
  const sidebarServerList = createMemo(() => gameServers().slice(0, 3));

  return {
    getCommunityFromUrl,
    getChannelFromUrl,
    selectedCommunityId,
    setSelectedCommunityId,
    selectedChannelId,
    setSelectedChannelId,
    showCreateCommunity,
    setShowCreateCommunity,
    showCreateChannel,
    setShowCreateChannel,
    showJoinCommunity,
    setShowJoinCommunity,
    showSettings,
    setShowSettings,
    showCreateCategory,
    setShowCreateCategory,
    showCreateEvent,
    setShowCreateEvent,
    showEvents,
    setShowEvents,
    showServers,
    setShowServers,
    renameTarget,
    setRenameTarget,
    renameCategoryTarget,
    setRenameCategoryTarget,
    showSearch,
    setShowSearch,
    searchInitialChannel,
    setSearchInitialChannel,
    replyTo,
    setReplyTo,
    pins,
    setPins,
    openedThread,
    activeThread,
    setActiveThread,
    activeThreadCount,
    editingTopic,
    setEditingTopic,
    topicDraft,
    setTopicDraft,
    editState,
    setEditState,
    isLoadingOlder,
    setIsLoadingOlder,
    hasMoreOlder,
    setHasMoreOlder,
    editingEvent,
    setEditingEvent,
    createPollTarget,
    setCreatePollTarget,
    createThreadTarget,
    setCreateThreadTarget,
    forwardTarget,
    setForwardTarget,
    showWelcomeForCommunity,
    setShowWelcomeForCommunity,
    showCallChat,
    setShowCallChat,
    rightPanel,
    setRightPanel,
    sidebarServersExpanded,
    setSidebarServersExpanded,
    sidebarEventsExpanded,
    setSidebarEventsExpanded,
    gameNameCache,
    setGameNameCache,
    activeCommunity,
    activeChannel,
    channelMessages,
    threadMessages,
    memberNames,
    memberName,
    myPerms,
    canManageChannels,
    canManageMessages,
    canRequestToSpeak,
    canManageCommunity,
    canSendMessages,
    canBypassSlowmode,
    gameServers,
    isAnnouncementChannel,
    isForumChannel,
    isStageChannel,
    isVoiceChannel,
    isTextPaneVisible,
    denied,
    ownRemoval,
    joinWithdrawn,
    channelLocked,
    keyPending,
    catchUpPending,
    canPostInChannel,
    activeVoiceChannelState,
    isConnectedToActiveStage,
    channelTypingUsers,
    upcomingEvents,
    shouldShowOnboarding,
    shouldShowWelcome,
    sidebarServerList,
    formatTimeUntilEvent,
  };
}
