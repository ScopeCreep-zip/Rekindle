import { subscribeBackendIntents } from "../handlers/intent.handlers";
import { Component, JSX, onMount, onCleanup, createSignal, Show } from "solid-js";
import { type UnlistenFn } from "@tauri-apps/api/event";
import SkinFrame from "../components/common/SkinFrame";
import SkinTreeWindow from "../components/skin-tree/SkinTreeWindow";
import { skinShell } from "../hooks/useSkin";
import Titlebar from "../components/titlebar/Titlebar";
import MenuBar from "../components/buddy-list/MenuBar";
import UserIdentityBar from "../components/buddy-list/UserIdentityBar";
import TabBar from "../components/buddy-list/TabBar";
import DownloadsTab from "../components/buddy-list/DownloadsTab";
import ServersView from "../components/skin-tree/ServersView";
import SearchBar, { focusSearchInput } from "../components/buddy-list/SearchBar";
import PendingRequests from "../components/buddy-list/PendingRequests";
import CoCreatorInvites from "../components/buddy-list/CoCreatorInvites";
import SuccessorOffers from "../components/buddy-list/SuccessorOffers";
import BuddyList from "../components/buddy-list/BuddyList";
import CommunityListCompact from "../components/buddy-list/CommunityListCompact";
import BottomActionBar from "../components/buddy-list/BottomActionBar";
import RouteStatus from "../components/buddy-list/RouteStatus";
import AddFriendModal from "../components/buddy-list/AddFriendModal";
import NewChatModal from "../components/buddy-list/NewChatModal";
import PairingLinkNotice from "../components/buddy-list/PairingLinkNotice";
import CreateCommunityModal from "../components/community/CreateCommunityModal";
import JoinCommunityModal from "../components/community/JoinCommunityModal";
import StartGroupCallModal from "../components/buddy-list/StartGroupCallModal";
import NewGroupModal from "../components/buddy-list/NewGroupModal";
import InviteToRoomModal from "../components/chat/InviteToRoomModal";
import GroupDmList from "../components/buddy-list/GroupDmList";
import { subscribeBuddyListGroupDmEvents } from "../handlers/group-dm.handlers";
import { subscribeBuddyListTransferEvents } from "../handlers/transfer.handlers";
import { handleLoadGroupDms } from "../actions/group-dm.actions";
import StatusPicker from "../components/status/StatusPicker";
import NetworkIndicator from "../components/status/NetworkIndicator";
import { authState, setAuthState } from "../stores/auth.store";
import { buddyListUI, setBuddyListUI } from "../stores/buddylist-ui.store";
import { switchTab } from "../stores/buddylist-ui.store";
import { handleLoadPendingRequests } from "../actions/buddy.actions";
import { handleGetGameStatus } from "../actions/settings.actions";
import { subscribeBuddyListChatEvents } from "../handlers/chat-events.handlers";
import { subscribeBuddyListPresenceEvents } from "../handlers/presence-events.handlers";
import { subscribeBuddyListVoiceEvents } from "../handlers/voice.handlers";
import { subscribeBuddyListCommunityEvents } from "../handlers/community/buddy_list_communities";
import { subscribePairingLinkHandler } from "../handlers/pairing-link.handler";
import { refreshMissedCalls, subscribeCallEvents } from "../handlers/calls.handlers";
import { subscribeNotificationHandler } from "../handlers/notification-events.handlers";
import CallController from "../components/voice/CallController";
import { startEventStream } from "../ipc/channels";
import { handleHydrateRelayState } from "../actions/relay.actions";
import { hydrateState } from "../stores/hydrate";
import {
  subscribeNetworkStatus,
  subscribeProfileUpdates,
  subscribeSkinSignals,
  subscribeSkinShell,
} from "../ipc/channels";
import { setSkinSignals } from "../stores/skin-signals.store";
import { registerShellRunner, runAddressedShellOp } from "../actions/skin.actions";
import type { ShellOp } from "../ipc/commands/dto_skin_run";
import { handleHide } from "../actions/titlebar.actions";
import { handleToggleAddFriend } from "../actions/buddy.actions";
import { setShortcutsHelpOpen } from "../stores/shortcuts_help.store";
import { handleToggleCreateCommunity, handleToggleJoinCommunity, handleToggleNewChat } from "../components/buddy-list/BottomActionBar";
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
  } else if (e.altKey && e.key === "3") {
    e.preventDefault();
    switchTab("servers");
  } else if (e.altKey && e.key === "4") {
    e.preventDefault();
    switchTab("files");
  } else if ((e.ctrlKey || e.metaKey) && e.key === "f") {
    e.preventDefault();
    focusSearchInput();
  }
}

/** The Friends lists: a skin's tree shows them in its Friends view (plan
 *  SK2-S2), the app's own layout under its Friends tab. */
function FriendsContent(): JSX.Element {
  return (
    <>
      <PendingRequests />
      <CoCreatorInvites />
      <SuccessorOffers />
      <GroupDmList />
      <BuddyList />
    </>
  );
}

/** The app's own layout (no skin tree): the shown tab's lists in
 *  `.window-main`. A tree switches its Friends and Communities views itself
 *  (the Host's ShowView, plan SK2-N4). */
function BuddyContent(): JSX.Element {
  return (
    <>
      <Show when={buddyListUI.activeTab === "friends"}>
        <FriendsContent />
      </Show>
      <Show when={buddyListUI.activeTab === "communities"}>
        <CommunityListCompact />
      </Show>
      <Show when={buddyListUI.activeTab === "servers"}>
        <ServersView />
      </Show>
      <Show when={buddyListUI.activeTab === "files"}>
        <DownloadsTab />
      </Show>
    </>
  );
}

/** The buddy list's own skin operations: Close hides it in the tray (it
 *  is never destroyed), and the panels are its dialogs. */
function runBuddyListOp(op: ShellOp): boolean {
  switch (op.kind) {
    case "close":
      handleHide();
      return true;
    case "openPanel":
      switch (op.arg) {
        case "addFriend":
          handleToggleAddFriend();
          return true;
        case "newChat":
          handleToggleNewChat();
          return true;
        case "newGroup":
          setBuddyListUI("showNewGroup", true);
          return true;
        case "startGroupCall":
          setBuddyListUI("showStartGroupCall", true);
          return true;
        case "createCommunity":
          handleToggleCreateCommunity();
          return true;
        case "joinCommunity":
          handleToggleJoinCommunity();
          return true;
        case "shortcuts":
          setShortcutsHelpOpen(true);
          return true;
        case "notifications":
          setBuddyListUI("showNotifications", true);
          return true;
      }
      return false;
    case "newRoom":
      // A room is a group DM (SK2-G §6): the New Group dialog, with the
      // selected friend the Host named already picked.
      setBuddyListUI({ newGroupPeer: op.arg.peer, showNewGroup: true });
      return true;
    case "inviteToRoom":
      // The Host answered a skin's Invite Users to Room for this group (G8).
      setBuddyListUI("inviteToRoom", op.arg.groupId);
      return true;
    default:
      return false;
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

  onMount(async () => {
    // Register event listeners FIRST so no events are missed during hydration.
    // The buddy list owns the device-wide UI: the call shell (ring, modal,
    // outgoing/group panels), OS notifications and the notification inbox.
    unlisteners.push(subscribeCallEvents({ owner: true }));
    unlisteners.push(subscribeNotificationHandler());
    unlisteners.push(subscribeBuddyListChatEvents());
    unlisteners.push(subscribeBuddyListGroupDmEvents());
    // The Downloads view's transfers (plan E7.2 G12): only this window lists them.
    unlisteners.push(subscribeBuddyListTransferEvents());
    unlisteners.push(subscribeBuddyListPresenceEvents());
    unlisteners.push(subscribeBuddyListVoiceEvents());
    // Each community's arrival, from the Host's one create and join path.
    unlisteners.push(subscribeBuddyListCommunityEvents());
    unlisteners.push(subscribeNetworkStatus((event: NetworkStatusEvent) => {
      setNetworkAttached(event.isAttached);
    }));
    unlisteners.push(subscribeProfileUpdates(handleProfileUpdated));
    // This window's skin signals: the tree applies the triggers they fire.
    unlisteners.push(subscribeSkinSignals(setSkinSignals));
    unlisteners.push(subscribeSkinShell(runAddressedShellOp));
    // The skin operations only the buddy list runs (plan SK2-A5): Close
    // hides it in the tray, and its dialogs.
    unlisteners.push(Promise.resolve(registerShellRunner(runBuddyListOp)));
    unlisteners.push(subscribePairingLinkHandler());
    // Reviews of actions the backend began, in the router root's dialog.
    unlisteners.push(subscribeBackendIntents());
    void startEventStream(() => void hydrateState());
    void refreshMissedCalls();

    // Await hydration so store is populated before subsequent commands
    await hydrateState();
    handleHydrateRelayState();

    // Load persisted pending friend requests from SQLite
    handleLoadPendingRequests();
    void handleLoadGroupDms();

    // Poll current game status on mount
    handleGetGameStatus().then((game) => {
      setAuthState("gameInfo", game);
    });

    // NOTE: the WebCodecs probe matrix deliberately does NOT run here.
    // The login path must never be the first media-stack touch — a cold
    // WebCodecs call aborts the WebKitWebProcess on WebKitGTK 2.52.3 +
    // GStreamer 1.24 (Ubuntu/Pop!_OS 24.04). The probe runs lazily from
    // handleJoinVoice; see handlers/video.handlers.ts.

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
    // `data-skin-window` names the skin's main frame layout (plan SK0 S2).
    <div class="app-frame buddy-list-window" data-skin-window="main">
      <SkinFrame />
      {/* Architecture §32 a11y — keyboard skip link past menu/identity rail. */}
      <a href="#main-content" class="skip-link">Skip to buddy list</a>
      {/* A skin that lays this window as its component tree (plan SK2-S2)
       *  draws it whole; without one (the built-in default) the app's own
       *  layout stays. */}
      <Show
        keyed
        when={skinShell("main")}
        fallback={
          <>
            <Titlebar title="Rekindle" hideOnClose />
            <MenuBar />
            <Show when={!networkAttached()}>
              <div class="network-banner">Connecting to Veilid network...</div>
            </Show>
            <UserIdentityBar />
            <TabBar />
            {/* Search filters the Friends and Communities lists only. */}
            <Show when={buddyListUI.activeTab === "friends" || buddyListUI.activeTab === "communities"}>
              <SearchBar />
            </Show>
            <PairingLinkNotice />
            <div id="main-content" tabindex="-1" class="window-main">
              <BuddyContent />
            </div>
            <BottomActionBar />
            {/* Our own route paused (owner decision 65) or down while the Host
                retries (plan F9-A), said here until the Host says it is up. */}
            <RouteStatus />
            <div class="status-bar">
              <StatusPicker currentStatus={authState.status} />
              <NetworkIndicator />
            </div>
          </>
        }
      >
        {(shell) => (
          <SkinTreeWindow
            shell={shell}
            views={() => (
              <>
                <SearchBar />
                <PairingLinkNotice />
                <FriendsContent />
              </>
            )}
            communities={() => <CommunityListCompact />}
          />
        )}
      </Show>
      <AddFriendModal />
      <NewChatModal />
      <CreateCommunityModal
        isOpen={buddyListUI.showCreateCommunity}
        onClose={() => setBuddyListUI("showCreateCommunity", false)}
      />
      <JoinCommunityModal
        isOpen={buddyListUI.showJoinCommunity}
        onClose={() => setBuddyListUI("showJoinCommunity", false)}
      />
      <CallController />
      <NewGroupModal />
      <InviteToRoomModal groupId={buddyListUI.inviteToRoom} onClose={() => setBuddyListUI("inviteToRoom", null)} />
      <StartGroupCallModal
        isOpen={buddyListUI.showStartGroupCall}
        onClose={() => setBuddyListUI("showStartGroupCall", false)}
      />
      {/* Wave 12 W12.1 — IncomingCallModal moved to <CallController />
       *  in main.tsx so it overlays the active window in every webview,
       *  not only the BuddyList. */}
    </div>
  );
};

export default BuddyListWindow;
