import { Component, createMemo, createSignal, For, Show } from "solid-js";
import { friendsState } from "../../stores/friends.store";
import { buddyListUI } from "../../stores/buddylist-ui.store";
import type { Friend } from "../../stores/friends.store";
import {
  handleDoubleClickFriend,
  handleContextMenuFriend,
  handleCloseContextMenu,
  handleRemoveFriend,
  handleCancelRequest,
  handleCreateFriendGroup,
  handleMoveFriendToGroup,
} from "../../handlers/buddy.handlers";
import { commands } from "../../ipc/commands";
import BuddyGroup from "./BuddyGroup";
import ScrollArea from "../common/ScrollArea";
import ContextMenu from "../common/ContextMenu";
import type { ContextMenuItem } from "../common/ContextMenu";

function matchesSearch(friend: Friend, query: string): boolean {
  const q = query.toLowerCase();
  if (friend.displayName.toLowerCase().includes(q)) return true;
  if (friend.nickname && friend.nickname.toLowerCase().includes(q)) return true;
  return false;
}

const BuddyList: Component = () => {
  const [selectedKey, setSelectedKey] = createSignal<string | null>(null);
  const [showGroupSubmenu, setShowGroupSubmenu] = createSignal(false);
  const [creatingGroup, setCreatingGroup] = createSignal(false);
  const [newGroupName, setNewGroupName] = createSignal("");

  const groupedFriends = createMemo(() => {
    const query = buddyListUI.searchQuery.trim();
    const groups: Record<string, Friend[]> = {};
    const pendingOut: Friend[] = [];
    for (const friend of Object.values(friendsState.friends)) {
      if (query && !matchesSearch(friend, query)) continue;
      if (friend.friendshipState === "pendingOut") {
        pendingOut.push(friend);
        continue;
      }
      const group = friend.group || "Friends";
      if (!groups[group]) groups[group] = [];
      groups[group].push(friend);
    }
    // Sort each group: online first, then away, then offline
    const statusOrder = { online: 0, away: 1, busy: 2, offline: 3 };
    for (const group of Object.values(groups)) {
      group.sort(
        (a, b) =>
          (statusOrder[a.status] ?? 3) - (statusOrder[b.status] ?? 3),
      );
    }
    if (pendingOut.length > 0) {
      groups["Awaiting Response"] = pendingOut;
    }
    return groups;
  });

  const hasFriends = createMemo(() => Object.keys(friendsState.friends).length > 0);
  const hasFilteredResults = createMemo(() => Object.keys(groupedFriends()).length > 0);

  const existingGroups = createMemo(() => {
    return Object.keys(groupedFriends());
  });

  function handleDblClick(publicKey: string, displayName: string): void {
    setSelectedKey(publicKey);
    handleDoubleClickFriend(publicKey, displayName);
  }

  function handleCtxMenu(e: MouseEvent, publicKey: string): void {
    setSelectedKey(publicKey);
    handleContextMenuFriend(e, publicKey);
    setShowGroupSubmenu(false);
    setCreatingGroup(false);
  }

  function handleCloseAllMenus(): void {
    handleCloseContextMenu();
    setShowGroupSubmenu(false);
    setCreatingGroup(false);
    setNewGroupName("");
  }

  function contextMenuItems(): ContextMenuItem[] {
    const key = friendsState.contextMenu?.publicKey;
    if (!key) return [];
    const friend = friendsState.friends[key];
    const name = friend?.displayName ?? key.slice(0, 12);

    // Pending-out friends get a different context menu
    if (friend?.friendshipState === "pendingOut") {
      return [
        {
          label: "Cancel Request",
          action: () => handleCancelRequest(key),
          danger: true,
        },
        {
          label: "Copy Public Key",
          action: () => navigator.clipboard.writeText(key),
        },
      ];
    }

    return [
      {
        label: "Chat",
        action: () => commands.openChatWindow(key, name),
      },
      {
        label: "View Profile",
        action: () => commands.openProfileWindow(key, name),
      },
      {
        label: "Move to Group",
        action: () => {
          setShowGroupSubmenu(true);
        },
      },
      {
        label: "Copy Public Key",
        action: () => navigator.clipboard.writeText(key),
      },
      {
        label: "Remove Friend",
        action: () => handleRemoveFriend(key),
        danger: true,
      },
    ];
  }

  async function handleMoveToExistingGroup(groupName: string): Promise<void> {
    const key = friendsState.contextMenu?.publicKey;
    if (!key) return;
    // We pass null for the default group or create/reuse groups.
    // The backend expects a group ID but our friends store only tracks group name.
    // Use createFriendGroup to get/create the ID, then move the friend.
    if (groupName === "Friends") {
      await handleMoveFriendToGroup(key, null);
    } else {
      const groupId = await handleCreateFriendGroup(groupName);
      if (groupId >= 0) {
        await handleMoveFriendToGroup(key, groupId);
      }
    }
    handleCloseAllMenus();
  }

  async function handleCreateAndMoveToGroup(): Promise<void> {
    const key = friendsState.contextMenu?.publicKey;
    const name = newGroupName().trim();
    if (!key || !name) return;
    const groupId = await handleCreateFriendGroup(name);
    if (groupId >= 0) {
      await handleMoveFriendToGroup(key, groupId);
    }
    handleCloseAllMenus();
  }

  return (
    <ScrollArea class="buddy-list">
      <Show when={hasFriends()} fallback={
        <div class="empty-placeholder">
          <div class="empty-placeholder-title">No Friends Yet</div>
          <div class="empty-placeholder-subtitle">Add a friend to get started</div>
        </div>
      }>
        <Show when={hasFilteredResults()} fallback={
          <div class="empty-placeholder">
            <div class="empty-placeholder-subtitle">No matches</div>
          </div>
        }>
          <For each={Object.entries(groupedFriends())}>
            {([name, friends]) => (
              <BuddyGroup
                name={name}
                friends={friends}
                selectedKey={selectedKey()}
                onDoubleClick={handleDblClick}
                onContextMenu={handleCtxMenu}
              />
            )}
          </For>
        </Show>
      </Show>
      <Show when={friendsState.contextMenu}>
        {(menu) => (
          <>
            <Show when={!showGroupSubmenu()}>
              <ContextMenu
                items={contextMenuItems()}
                x={menu().x}
                y={menu().y}
                onClose={handleCloseAllMenus}
              />
            </Show>
            <Show when={showGroupSubmenu()}>
              <div
                class="context-menu"
                style={{
                  left: `${menu().x}px`,
                  top: `${menu().y}px`,
                }}
              >
                <div class="context-menu-header">Move to Group</div>
                <For each={existingGroups()}>
                  {(group) => (
                    <div
                      class="group-submenu-item"
                      onClick={() => handleMoveToExistingGroup(group)}
                    >
                      {group}
                    </div>
                  )}
                </For>
                <div class="context-menu-separator" />
                <Show when={!creatingGroup()}>
                  <div
                    class="context-menu-item"
                    onClick={() => setCreatingGroup(true)}
                  >
                    + New Group
                  </div>
                </Show>
                <Show when={creatingGroup()}>
                  <div class="group-create-inline">
                    <input
                      class="group-create-input"
                      type="text"
                      placeholder="Group name"
                      value={newGroupName()}
                      onInput={(e) => setNewGroupName(e.currentTarget.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") handleCreateAndMoveToGroup();
                        if (e.key === "Escape") setCreatingGroup(false);
                      }}
                      autofocus
                    />
                    <button
                      class="group-create-ok-btn"
                      onClick={handleCreateAndMoveToGroup}
                      disabled={!newGroupName().trim()}
                    >
                      OK
                    </button>
                  </div>
                </Show>
                <div class="context-menu-separator" />
                <div
                  class="context-menu-item"
                  onClick={handleCloseAllMenus}
                >
                  Cancel
                </div>
              </div>
            </Show>
          </>
        )}
      </Show>
    </ScrollArea>
  );
};

export default BuddyList;
