import { type Component, Show, createSignal } from "solid-js";
import { handleToggleAddFriend } from "../../actions/buddy.actions";
import { handleLogout } from "../../actions/auth.actions";
import { setFriendsState } from "../../stores/friends.store";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";
import { canCreateCommunity, networkLabel, trackNetworkStatus } from "../../stores/network.store";
import {
  ICON_NEW_CHAT,
  ICON_ADD_FRIEND,
  ICON_PLUS,
  ICON_COMMUNITIES,
  ICON_LOGOUT,
  ICON_KEYBOARD,
} from "../../icons";
import { setShortcutsHelpOpen } from "../../stores/shortcuts_help.store";

export function handleToggleNewChat(): void {
  setFriendsState("showNewChat", (prev) => !prev);
}

export function handleToggleCreateCommunity(): void {
  setBuddyListUI("showCreateCommunity", (prev) => !prev);
}

export function handleToggleJoinCommunity(): void {
  setBuddyListUI("showJoinCommunity", (prev) => !prev);
}

const BottomActionBar: Component = () => {
  // A community create cannot be held for later (its first record's key is
  // destroyed once written, ADR 0011), so Create waits for the Host to take
  // writes with the DHT online (plan E7.13, `canCreateCommunity`).
  trackNetworkStatus();
  const createTitle = (): string =>
    canCreateCommunity()
      ? "Create Community"
      : `Create Community (waiting for the network: ${networkLabel()})`;
  // The backend refuses a second concurrent logout; the button reflects
  // the one in flight.
  const [loggingOut, setLoggingOut] = createSignal(false);
  const logout = async (): Promise<void> => {
    setLoggingOut(true);
    try {
      await handleLogout();
    } finally {
      setLoggingOut(false);
    }
  };
  return (
    <div class="action-bar">
      <Show when={buddyListUI.activeTab === "friends"}>
        <button
          type="button"
          class="action-bar-icon-btn"
          onClick={handleToggleNewChat}
          title="New Chat"
          aria-label="New Chat"
        >
          <span class="nf-icon" aria-hidden="true">
            {ICON_NEW_CHAT}
          </span>
        </button>
        <button
          type="button"
          class="action-bar-icon-btn"
          onClick={handleToggleAddFriend}
          title="Add Friend"
          aria-label="Add Friend"
        >
          <span class="nf-icon" aria-hidden="true">
            {ICON_ADD_FRIEND}
          </span>
        </button>
        <button
          type="button"
          class="action-bar-icon-btn"
          onClick={() => setBuddyListUI("showNewGroup", true)}
          title="New Group"
          aria-label="New Group"
        >
          <span class="nf-icon" aria-hidden="true">
            {ICON_PLUS}
          </span>
        </button>
      </Show>
      <Show when={buddyListUI.activeTab === "communities"}>
        <button
          type="button"
          class="action-bar-icon-btn"
          onClick={handleToggleCreateCommunity}
          disabled={!canCreateCommunity()}
          title={createTitle()}
          aria-label="Create Community"
        >
          <span class="nf-icon" aria-hidden="true">
            {ICON_PLUS}
          </span>
        </button>
        <button
          type="button"
          class="action-bar-icon-btn"
          onClick={handleToggleJoinCommunity}
          title="Join Community"
          aria-label="Join Community"
        >
          <span class="nf-icon" aria-hidden="true">
            {ICON_COMMUNITIES}
          </span>
        </button>
      </Show>
      <div class="action-bar-spacer" />
      <button
        type="button"
        class="action-bar-icon-btn"
        onClick={() => setShortcutsHelpOpen(true)}
        title="Keyboard shortcuts (Ctrl+/ or Cmd+/)"
        aria-label="Keyboard shortcuts"
      >
        <span class="nf-icon" aria-hidden="true">
          {ICON_KEYBOARD}
        </span>
      </button>
      <button
        type="button"
        class="logout-icon-btn"
        onClick={logout}
        disabled={loggingOut()}
        title="Logout"
        aria-label="Logout"
      >
        <span class="nf-icon" aria-hidden="true">
          {ICON_LOGOUT}
        </span>
      </button>
    </div>
  );
};

export default BottomActionBar;
