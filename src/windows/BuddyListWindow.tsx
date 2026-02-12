import { Component, onMount, onCleanup, createSignal, Show } from "solid-js";
import { type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import UserIdentityBar from "../components/buddy-list/UserIdentityBar";
import PendingRequests from "../components/buddy-list/PendingRequests";
import BuddyList from "../components/buddy-list/BuddyList";
import BottomActionBar from "../components/buddy-list/BottomActionBar";
import AddFriendModal from "../components/buddy-list/AddFriendModal";
import NewChatModal from "../components/buddy-list/NewChatModal";
import StatusPicker from "../components/status/StatusPicker";
import NetworkIndicator from "../components/status/NetworkIndicator";
import { authState, setAuthState } from "../stores/auth.store";
import { handleRefreshFriends, handleLoadPendingRequests } from "../handlers/buddy.handlers";
import { handleGetGameStatus } from "../handlers/settings.handlers";
import { hydrateState } from "../ipc/hydrate";
import {
  subscribeChatEvents,
  subscribePresenceEvents,
  subscribeNotificationEvents,
  subscribeVoiceEvents,
  subscribeNetworkStatus,
  subscribeProfileUpdates,
} from "../ipc/channels";
import { handleTypingIndicator } from "../handlers/chat.handlers";
import { fetchAvatarUrl } from "../ipc/avatar";
import { commands } from "../ipc/commands";
import { setFriendsState, friendsState } from "../stores/friends.store";
import { setNotificationState } from "../stores/notification.store";
import type { UserStatus } from "../stores/auth.store";
import type { ChatEvent, PresenceEvent, NotificationEvent, VoiceEvent, NetworkStatusEvent } from "../ipc/channels";

function handleChatEvent(event: ChatEvent): void {
  switch (event.type) {
    case "MessageReceived": {
      // Buddy list only tracks unread counts — chat windows handle full messages
      const senderId = event.data.from;
      if (friendsState.friends[senderId]) {
        setFriendsState("friends", senderId, "unreadCount", (c) => (c ?? 0) + 1);
      }
      break;
    }
    case "TypingIndicator": {
      handleTypingIndicator(event.data.from, event.data.typing);
      break;
    }
    case "FriendRequest": {
      const exists = friendsState.pendingRequests.some(
        (r) => r.publicKey === event.data.from,
      );
      if (!exists) {
        setFriendsState("pendingRequests", (reqs) => [
          ...reqs,
          {
            publicKey: event.data.from,
            displayName: event.data.displayName,
            message: event.data.message,
          },
        ]);
      }
      break;
    }
    case "FriendRequestAccepted": {
      handleRefreshFriends();
      break;
    }
    case "FriendAdded": {
      // A friend was added to our list — update the store
      setFriendsState("friends", event.data.publicKey, {
        publicKey: event.data.publicKey,
        displayName: event.data.displayName,
        nickname: null,
        status: "offline" as const,
        statusMessage: null,
        gameInfo: null,
        group: "Friends",
        unreadCount: 0,
        lastSeenAt: null,
        voiceChannel: null,
      });
      break;
    }
    case "FriendRequestRejected": {
      const truncatedKey = event.data.from.slice(0, 8);
      setNotificationState("notifications", (prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          type: "system",
          title: "Friend Request Declined",
          body: `Your friend request was declined by ${truncatedKey}...`,
          timestamp: Date.now(),
          read: false,
        },
      ]);
      setNotificationState("unreadCount", (c) => c + 1);
      break;
    }
  }
}

function handlePresenceEvent(event: PresenceEvent): void {
  switch (event.type) {
    case "FriendOnline": {
      setFriendsState("friends", event.data.publicKey, "status", "online");
      break;
    }
    case "FriendOffline": {
      setFriendsState("friends", event.data.publicKey, "status", "offline");
      setFriendsState("friends", event.data.publicKey, "lastSeenAt", Date.now());
      break;
    }
    case "StatusChanged": {
      setFriendsState(
        "friends",
        event.data.publicKey,
        "status",
        event.data.status as UserStatus,
      );
      if (event.data.statusMessage !== undefined) {
        setFriendsState(
          "friends",
          event.data.publicKey,
          "statusMessage",
          event.data.statusMessage,
        );
      }
      break;
    }
    case "GameChanged": {
      if (event.data.gameName) {
        setFriendsState("friends", event.data.publicKey, "gameInfo", {
          gameName: event.data.gameName,
          gameId: event.data.gameId,
          startedAt: event.data.elapsedSeconds,
        });
      } else {
        setFriendsState("friends", event.data.publicKey, "gameInfo", null);
      }
      break;
    }
  }
}

function handleVoiceEvent(event: VoiceEvent): void {
  switch (event.type) {
    case "UserJoined": {
      // Mark the friend as being in a voice channel
      if (friendsState.friends[event.data.publicKey]) {
        setFriendsState("friends", event.data.publicKey, "voiceChannel", "active");
      }
      break;
    }
    case "UserLeft": {
      if (friendsState.friends[event.data.publicKey]) {
        setFriendsState("friends", event.data.publicKey, "voiceChannel", null);
      }
      break;
    }
  }
}

function handleNotificationEvent(event: NotificationEvent): void {
  switch (event.type) {
    case "SystemAlert": {
      setNotificationState("notifications", (prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          type: "system",
          title: event.data.title,
          body: event.data.body,
          timestamp: Date.now(),
          read: false,
        },
      ]);
      setNotificationState("unreadCount", (c) => c + 1);
      break;
    }
    case "UpdateAvailable": {
      setNotificationState("notifications", (prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          type: "system",
          title: "Update Available",
          body: `Version ${event.data.version} is available`,
          timestamp: Date.now(),
          read: false,
        },
      ]);
      setNotificationState("unreadCount", (c) => c + 1);
      break;
    }
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
    unlisteners.push(subscribeChatEvents(handleChatEvent));
    unlisteners.push(subscribePresenceEvents(handlePresenceEvent));
    unlisteners.push(subscribeNotificationEvents(handleNotificationEvent));
    unlisteners.push(subscribeVoiceEvents(handleVoiceEvent));
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
  });

  onCleanup(() => {
    for (const p of unlisteners) {
      p.then((unlisten) => unlisten());
    }
  });

  return (
    <div class="app-frame">
      <Titlebar title="Rekindle" hideOnClose />
      <Show when={!networkAttached()}>
        <div class="network-banner">Connecting to Veilid network...</div>
      </Show>
      <UserIdentityBar />
      <PendingRequests />
      <BuddyList />
      <BottomActionBar />
      <div class="status-bar">
        <StatusPicker currentStatus={authState.status} />
        <NetworkIndicator />
      </div>
      <AddFriendModal />
      <NewChatModal />
    </div>
  );
};

export default BuddyListWindow;
