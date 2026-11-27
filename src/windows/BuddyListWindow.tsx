import type { UnlistenFn } from "@tauri-apps/api/event";
import { type Component, createSignal, type JSX, onCleanup, onMount, Show } from "solid-js";
import { handleLoadPendingRequests, handleToggleAddFriend } from "../actions/buddy.actions";
import { handleLoadGroupDms } from "../actions/group-dm.actions";
import { handleHydrateRelayState } from "../actions/relay.actions";
import { handleGetGameStatus } from "../actions/settings.actions";
import {
  chatOpenOps,
  friendOps,
  mainOps,
  menuOps,
  muteOps,
  popupOps,
  transferOps,
} from "../actions/shell_runners";
import { registerShellRunner, runAddressedShellOp } from "../actions/skin.actions";
import { handleHide } from "../actions/titlebar.actions";
import AddFriendModal from "../components/buddy-list/AddFriendModal";
import BottomActionBar, {
  handleToggleCreateCommunity,
  handleToggleJoinCommunity,
  handleToggleNewChat,
} from "../components/buddy-list/BottomActionBar";
import BuddyList from "../components/buddy-list/BuddyList";
import CoCreatorInvites from "../components/buddy-list/CoCreatorInvites";
import CommunityListCompact from "../components/buddy-list/CommunityListCompact";
import CommunityStartModal from "../components/buddy-list/CommunityStartModal";
import DownloadsTab from "../components/buddy-list/DownloadsTab";
import GroupDmList from "../components/buddy-list/GroupDmList";
import MenuBar from "../components/buddy-list/MenuBar";
import NewChatModal from "../components/buddy-list/NewChatModal";
import NewGroupModal from "../components/buddy-list/NewGroupModal";
import NotificationPanel from "../components/buddy-list/NotificationPanel";
import PairingLinkNotice from "../components/buddy-list/PairingLinkNotice";
import PendingRequests from "../components/buddy-list/PendingRequests";
import RouteStatus from "../components/buddy-list/RouteStatus";
import SearchBar, { focusSearchInput } from "../components/buddy-list/SearchBar";
import StartGroupCallModal from "../components/buddy-list/StartGroupCallModal";
import SuccessorOffers from "../components/buddy-list/SuccessorOffers";
import TabBar from "../components/buddy-list/TabBar";
import UserIdentityBar from "../components/buddy-list/UserIdentityBar";
import InviteToRoomModal from "../components/chat/InviteToRoomModal";
import SkinFrame from "../components/common/SkinFrame";
import CreateCommunityModal from "../components/community/CreateCommunityModal";
import JoinCommunityModal from "../components/community/JoinCommunityModal";
import Infoview from "../components/infoview/Infoview";
import FilesView from "../components/skin-tree/FilesView";
import NameField from "../components/skin-tree/NameField";
import ServersView from "../components/skin-tree/ServersView";
import SkinTreeWindow from "../components/skin-tree/SkinTreeWindow";
import { StatusList, StatusText } from "../components/skin-tree/StatusField";
import NetworkIndicator from "../components/status/NetworkIndicator";
import StatusPicker from "../components/status/StatusPicker";
import Titlebar from "../components/titlebar/Titlebar";
import CallController from "../components/voice/CallController";
import { refreshMissedCalls, subscribeCallEvents } from "../handlers/calls.handlers";
import { subscribeBuddyListChatEvents } from "../handlers/chat-events.handlers";
import { subscribeBuddyListCommunityEvents } from "../handlers/community/buddy_list_communities";
import { subscribeBuddyListGroupDmEvents } from "../handlers/group-dm.handlers";
import { subscribeBackendIntents } from "../handlers/intent.handlers";
import { subscribeNotificationHandler } from "../handlers/notification-events.handlers";
import { subscribePairingLinkHandler } from "../handlers/pairing-link.handler";
import { subscribeBuddyListPresenceEvents } from "../handlers/presence-events.handlers";
import { subscribeBuddyListTransferEvents } from "../handlers/transfer.handlers";
import { subscribeBuddyListVoiceEvents } from "../handlers/voice.handlers";
import { skinShell } from "../hooks/useSkin";
import { fetchAvatarUrl } from "../ipc/avatar";
import type { NetworkStatusEvent } from "../ipc/channels";
import {
  startEventStream,
  subscribeNetworkStatus,
  subscribeProfileUpdates,
  subscribeSkinShell,
  subscribeSkinSignals,
} from "../ipc/channels";
import { commands } from "../ipc/commands";
import type { ShellOp } from "../ipc/commands/dto_skin_run";
import { authState, setAuthState } from "../stores/auth.store";
import { buddyListUI, setBuddyListUI, switchTab } from "../stores/buddylist-ui.store";
import { hydrateState } from "../stores/hydrate";
import { setShortcutsHelpOpen } from "../stores/shortcuts_help.store";
import { setSkinSignals } from "../stores/skin-signals.store";
import { setWindowKind } from "../utils/window_kind";

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
  setWindowKind("buddy-list");
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
        case "communityStart":
          setBuddyListUI("showCommunityStart", true);
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
    unlisteners.push(
      subscribeNetworkStatus((event: NetworkStatusEvent) => {
        setNetworkAttached(event.isAttached);
      })
    );
    unlisteners.push(subscribeProfileUpdates(handleProfileUpdated));
    // This window's skin signals: the tree applies the triggers they fire.
    unlisteners.push(subscribeSkinSignals(setSkinSignals));
    unlisteners.push(subscribeSkinShell(runAddressedShellOp));
    // The skin operations only the buddy list runs (plan SK2-A5): Close
    // hides it in the tray, and its dialogs.
    unlisteners.push(Promise.resolve(registerShellRunner(runBuddyListOp)));
    // The groups the main window's trees serve (shell_runners): its menus,
    // popups, a friend's profile and chat, a room, downloads, Copy Key and
    // Show Voice. No call or mute: no main tree lays one.
    for (const run of [menuOps, popupOps, friendOps, chatOpenOps, transferOps, mainOps, muteOps]) {
      unlisteners.push(Promise.resolve(registerShellRunner(run)));
    }
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
      <a href="#main-content" class="skip-link">
        Skip to buddy list
      </a>
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
            <Show
              when={buddyListUI.activeTab === "friends" || buddyListUI.activeTab === "communities"}
            >
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
            pieces={{
              statusText: StatusText,
              statusList: StatusList,
              nameField: NameField,
              infoView: () => <Infoview subject={{ type: "selection" }} />,
              servers: () => <ServersView />,
              files: (onExecute) => <FilesView onExecute={onExecute} />,
            }}
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
      {/* The notification inbox, in the stock layout and under every pack. */}
      <NotificationPanel />
      <NewGroupModal />
      <InviteToRoomModal
        groupId={buddyListUI.inviteToRoom}
        onClose={() => setBuddyListUI("inviteToRoom", null)}
      />
      <CommunityStartModal
        isOpen={buddyListUI.showCommunityStart}
        onClose={() => setBuddyListUI("showCommunityStart", false)}
      />
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
