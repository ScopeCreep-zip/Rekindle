import { Component, onMount, onCleanup, createSignal, Show } from "solid-js";
import { type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import MenuBar from "../components/buddy-list/MenuBar";
import UserIdentityBar from "../components/buddy-list/UserIdentityBar";
import TabBar from "../components/buddy-list/TabBar";
import SearchBar, { focusSearchInput } from "../components/buddy-list/SearchBar";
import PendingRequests from "../components/buddy-list/PendingRequests";
import BuddyList from "../components/buddy-list/BuddyList";
import CommunityListCompact from "../components/buddy-list/CommunityListCompact";
import BottomActionBar from "../components/buddy-list/BottomActionBar";
import AddFriendModal from "../components/buddy-list/AddFriendModal";
import NewChatModal from "../components/buddy-list/NewChatModal";
import BuddyCreateCommunityModal from "../components/buddy-list/BuddyCreateCommunityModal";
import BuddyJoinCommunityModal from "../components/buddy-list/BuddyJoinCommunityModal";
import StatusPicker from "../components/status/StatusPicker";
import NetworkIndicator from "../components/status/NetworkIndicator";
import { authState, setAuthState } from "../stores/auth.store";
import { buddyListUI } from "../stores/buddylist-ui.store";
import { switchTab } from "../stores/buddylist-ui.store";
import { handleLoadPendingRequests } from "../handlers/buddy.handlers";
import { handleGetGameStatus } from "../handlers/settings.handlers";
import { subscribeBuddyListChatEvents } from "../handlers/chat-events.handlers";
import { subscribeBuddyListPresenceEvents } from "../handlers/presence-events.handlers";
import { subscribeNotificationHandler } from "../handlers/notification-events.handlers";
import { subscribeBuddyListVoiceEvents } from "../handlers/voice.handlers";
import { hydrateState } from "../ipc/hydrate";
import {
  subscribeNetworkStatus,
  subscribeProfileUpdates,
} from "../ipc/channels";
import { fetchAvatarUrl } from "../ipc/avatar";
import { commands } from "../ipc/commands";
import type { NetworkStatusEvent } from "../ipc/channels";

function handleKeyboardShortcuts(e: KeyboardEvent): void {
  if (e.altKey && e.key === "1") {
    e.preventDefault();
    switchTab("friends");
  } else if (e.altKey && e.key === "2") {
    e.preventDefault();
    switchTab("communities");
  } else if ((e.ctrlKey || e.metaKey) && e.key === "f") {
    e.preventDefault();
    focusSearchInput();
  }
}

const BuddyListWindow: Component = () => {
  const [networkAttached, setNetworkAttached] = createSignal(true);
  const unlisteners: Promise<UnlistenFn>[] = [];

  async function handleProfileUpdated(): Promise<void> {
    const identity = await commands.getIdentity();
    if (identity) {
      setAuthState("displayName", identity.displayName);
      const avatarUrl = await fetchAvatarUrl(identity.publicKey);
      setAuthState("avatarUrl", avatarUrl);
    }
  }

  onMount(() => {
    hydrateState();
    unlisteners.push(subscribeBuddyListChatEvents());
    unlisteners.push(subscribeBuddyListPresenceEvents());
    unlisteners.push(subscribeNotificationHandler());
    unlisteners.push(subscribeBuddyListVoiceEvents());
    unlisteners.push(subscribeNetworkStatus((event: NetworkStatusEvent) => {
      setNetworkAttached(event.isAttached);
    }));
    unlisteners.push(subscribeProfileUpdates(handleProfileUpdated));

    // Load persisted pending friend requests from SQLite
    handleLoadPendingRequests();

    // Poll current game status on mount
    handleGetGameStatus().then((game) => {
      setAuthState("gameInfo", game);
    });

    // Keyboard shortcuts
    document.addEventListener("keydown", handleKeyboardShortcuts);
  });

  onCleanup(() => {
    for (const p of unlisteners) {
      p.then((unlisten) => unlisten());
    }
    document.removeEventListener("keydown", handleKeyboardShortcuts);
  });

  return (
    <div class="app-frame">
      <Titlebar title="Rekindle" hideOnClose />
      <MenuBar />
      <Show when={!networkAttached()}>
        <div class="network-banner">Connecting to Veilid network...</div>
      </Show>
      <UserIdentityBar />
      <TabBar />
      <SearchBar />
      <Show when={buddyListUI.activeTab === "friends"}>
        <PendingRequests />
        <BuddyList />
      </Show>
      <Show when={buddyListUI.activeTab === "communities"}>
        <CommunityListCompact />
      </Show>
      <BottomActionBar />
      <div class="status-bar">
        <StatusPicker currentStatus={authState.status} />
        <NetworkIndicator />
      </div>
      <AddFriendModal />
      <NewChatModal />
      <BuddyCreateCommunityModal />
      <BuddyJoinCommunityModal />
    </div>
  );
};

export default BuddyListWindow;
