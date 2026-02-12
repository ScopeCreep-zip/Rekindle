import { Component } from "solid-js";
import { handleToggleAddFriend } from "../../handlers/buddy.handlers";
import { handleLogout } from "../../handlers/auth.handlers";
import { setFriendsState } from "../../stores/friends.store";
import { commands } from "../../ipc/commands";
import { ICON_NEW_CHAT, ICON_ADD_FRIEND, ICON_COMMUNITIES, ICON_SETTINGS, ICON_LOGOUT } from "../../icons";

function handleToggleNewChat(): void {
  setFriendsState("showNewChat", (prev) => !prev);
}

const BottomActionBar: Component = () => {
  return (
    <div class="action-bar">
      <button class="action-bar-icon-btn" onClick={handleToggleNewChat} title="New Chat">
        <span class="nf-icon">{ICON_NEW_CHAT}</span>
      </button>
      <button class="action-bar-icon-btn" onClick={handleToggleAddFriend} title="Add Friend">
        <span class="nf-icon">{ICON_ADD_FRIEND}</span>
      </button>
      <button
        class="action-bar-icon-btn"
        onClick={() => commands.openCommunityWindow("", "Communities")}
        title="Communities"
      >
        <span class="nf-icon">{ICON_COMMUNITIES}</span>
      </button>
      <button
        class="action-bar-icon-btn"
        onClick={() => commands.openSettingsWindow()}
        title="Settings"
      >
        <span class="nf-icon">{ICON_SETTINGS}</span>
      </button>
      <button class="logout-icon-btn" onClick={handleLogout} title="Logout">
        <span class="nf-icon">{ICON_LOGOUT}</span>
      </button>
    </div>
  );
};

export default BottomActionBar;
