import { Component, Show } from "solid-js";
import { handleToggleAddFriend } from "../../handlers/buddy.handlers";
import { handleLogout } from "../../handlers/auth.handlers";
import { setFriendsState } from "../../stores/friends.store";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";
import { ICON_NEW_CHAT, ICON_ADD_FRIEND, ICON_PLUS, ICON_COMMUNITIES, ICON_LOGOUT } from "../../icons";

function handleToggleNewChat(): void {
  setFriendsState("showNewChat", (prev) => !prev);
}

function handleToggleCreateCommunity(): void {
  setBuddyListUI("showCreateCommunity", (prev) => !prev);
}

function handleToggleJoinCommunity(): void {
  setBuddyListUI("showJoinCommunity", (prev) => !prev);
}

const BottomActionBar: Component = () => {
  return (
    <div class="action-bar">
      <Show when={buddyListUI.activeTab === "friends"}>
        <button class="action-bar-icon-btn" onClick={handleToggleNewChat} title="New Chat">
          <span class="nf-icon">{ICON_NEW_CHAT}</span>
        </button>
        <button class="action-bar-icon-btn" onClick={handleToggleAddFriend} title="Add Friend">
          <span class="nf-icon">{ICON_ADD_FRIEND}</span>
        </button>
      </Show>
      <Show when={buddyListUI.activeTab === "communities"}>
        <button class="action-bar-icon-btn" onClick={handleToggleCreateCommunity} title="Create Community">
          <span class="nf-icon">{ICON_PLUS}</span>
        </button>
        <button class="action-bar-icon-btn" onClick={handleToggleJoinCommunity} title="Join Community">
          <span class="nf-icon">{ICON_COMMUNITIES}</span>
        </button>
      </Show>
      <div class="action-bar-spacer" />
      <button class="logout-icon-btn" onClick={handleLogout} title="Logout">
        <span class="nf-icon">{ICON_LOGOUT}</span>
      </button>
    </div>
  );
};

export default BottomActionBar;
