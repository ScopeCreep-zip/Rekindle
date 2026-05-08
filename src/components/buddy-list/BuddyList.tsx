import { Component, createMemo, createSignal, For, JSX, Show } from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import { friendsState } from "../../stores/friends.store";
import { buddyListUI } from "../../stores/buddylist-ui.store";
import type { Friend } from "../../stores/friends.store";
import {
  handleDoubleClickFriend,
  handleRemoveFriend,
  handleCancelRequest,
  handleCreateFriendGroup,
  handleMoveFriendToGroup,
  handleBlockUser,
  handleResetSignalSession,
} from "../../handlers/buddy.handlers";
import {
  handleRevokeRelay,
  handleVolunteerRelay,
  relayState,
} from "../../handlers/relay.handlers";
import { handleStartDm } from "../../handlers/dm.handlers";
import { handleStartDmCall } from "../../handlers/calls.handlers";
import { authState } from "../../stores/auth.store";
import { commands } from "../../ipc/commands";
import BuddyGroup from "./BuddyGroup";
import ScrollArea from "../common/ScrollArea";
import SimpleInputModal from "../common/SimpleInputModal";

function matchesSearch(friend: Friend, query: string): boolean {
  const q = query.toLowerCase();
  if (friend.displayName.toLowerCase().includes(q)) return true;
  if (friend.nickname && friend.nickname.toLowerCase().includes(q)) return true;
  return false;
}

const BuddyList: Component = () => {
  const [selectedKey, setSelectedKey] = createSignal<string | null>(null);
  const [newGroupTarget, setNewGroupTarget] = createSignal<string | null>(null);

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

  const existingGroups = createMemo(() => Object.keys(groupedFriends()));

  function handleDblClick(publicKey: string, displayName: string): void {
    setSelectedKey(publicKey);
    handleDoubleClickFriend(publicKey, displayName);
  }

  function handleSelectFriend(publicKey: string): void {
    setSelectedKey(publicKey);
  }

  async function moveToExistingGroup(friendKey: string, groupName: string): Promise<void> {
    if (groupName === "Friends") {
      await handleMoveFriendToGroup(friendKey, null);
    } else {
      const groupId = await handleCreateFriendGroup(groupName);
      if (groupId >= 0) {
        await handleMoveFriendToGroup(friendKey, groupId);
      }
    }
  }

  async function createGroupAndMove(friendKey: string, groupName: string): Promise<void> {
    const trimmed = groupName.trim();
    if (!trimmed) return;
    const groupId = await handleCreateFriendGroup(trimmed);
    if (groupId >= 0) {
      await handleMoveFriendToGroup(friendKey, groupId);
    }
  }

  function buddyMenu(friend: Friend): JSX.Element {
    const key = friend.publicKey;
    const name = friend.displayName ?? key.slice(0, 12);

    if (friend.friendshipState === "pendingOut") {
      return (
        <ContextMenu.Portal>
          <ContextMenu.Content class="context-menu">
            <ContextMenu.Item
              class="context-menu-item context-menu-item-danger"
              onSelect={() => handleCancelRequest(key)}
            >
              Cancel Request
            </ContextMenu.Item>
            <ContextMenu.Item
              class="context-menu-item context-menu-item-danger"
              onSelect={() => handleBlockUser(key, friend.displayName)}
            >
              Block
            </ContextMenu.Item>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => navigator.clipboard.writeText(key)}
            >
              Copy Public Key
            </ContextMenu.Item>
          </ContextMenu.Content>
        </ContextMenu.Portal>
      );
    }

    return (
      <ContextMenu.Portal>
        <ContextMenu.Content class="context-menu">
          <ContextMenu.Item
            class="context-menu-item"
            onSelect={() => commands.openChatWindow(key, name)}
          >
            Chat
          </ContextMenu.Item>
          {/* Wave 12 W12.8 — quick-call entries so the user doesn't have
           *  to enter a DM window first. handleStartDmCall seeds the
           *  outgoingCall store entry; OutgoingCallPanel (mounted in
           *  CallController) surfaces the "Calling…" UX globally. */}
          <ContextMenu.Item
            class="context-menu-item"
            onSelect={() => void handleStartDmCall(key, name, false)}
          >
            Voice Call
          </ContextMenu.Item>
          <ContextMenu.Item
            class="context-menu-item"
            onSelect={() => void handleStartDmCall(key, name, true)}
          >
            Video Call
          </ContextMenu.Item>
          <ContextMenu.Item
            class="context-menu-item"
            onSelect={() => {
              const myName = authState.displayName ?? "Me";
              handleStartDm(key, myName);
            }}
          >
            Start DM (architecture §27)
          </ContextMenu.Item>
          <ContextMenu.Item
            class="context-menu-item"
            onSelect={() => commands.openProfileWindow(key, name)}
          >
            View Profile
          </ContextMenu.Item>

          <Show when={friend.gameInfo?.serverAddress && friend.gameInfo?.gameId}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() =>
                commands.launchGameToServer(
                  friend.gameInfo!.gameId!,
                  friend.gameInfo!.serverAddress!,
                )
              }
            >
              Join Game
            </ContextMenu.Item>
          </Show>

          <ContextMenu.Sub>
            <ContextMenu.SubTrigger class="context-menu-item">
              Move to Group
            </ContextMenu.SubTrigger>
            <ContextMenu.Portal>
              <ContextMenu.SubContent class="context-menu">
                <For each={existingGroups()}>
                  {(group) => (
                    <ContextMenu.Item
                      class="context-menu-item"
                      onSelect={() => void moveToExistingGroup(key, group)}
                    >
                      {group}
                    </ContextMenu.Item>
                  )}
                </For>
                <ContextMenu.Separator class="context-menu-separator" />
                <ContextMenu.Item
                  class="context-menu-item"
                  onSelect={() => setNewGroupTarget(key)}
                >
                  + New Group…
                </ContextMenu.Item>
              </ContextMenu.SubContent>
            </ContextMenu.Portal>
          </ContextMenu.Sub>

          <ContextMenu.Item
            class="context-menu-item"
            onSelect={() =>
              relayState.volunteeredFor[key]
                ? handleRevokeRelay(key)
                : handleVolunteerRelay(key)
            }
          >
            {relayState.volunteeredFor[key]
              ? "Stop relaying for this friend"
              : "Volunteer to relay for this friend"}
          </ContextMenu.Item>

          <ContextMenu.Item
            class="context-menu-item"
            onSelect={() => navigator.clipboard.writeText(key)}
          >
            Copy Public Key
          </ContextMenu.Item>

          {/* B6 — explicit Signal session reset. Confirms with the user
            * because the safety stance forbids auto-rehandshake; the
            * user is expected to verify the peer's safety number
            * out-of-band before resuming sensitive conversations on
            * the new session. */}
          <ContextMenu.Item
            class="context-menu-item"
            onSelect={() => void handleResetSignalSession(key, name)}
          >
            Reset Secure Session
          </ContextMenu.Item>

          <ContextMenu.Separator class="context-menu-separator" />

          <ContextMenu.Item
            class="context-menu-item context-menu-item-danger"
            onSelect={() => handleRemoveFriend(key)}
          >
            Remove Friend
          </ContextMenu.Item>
          <ContextMenu.Item
            class="context-menu-item context-menu-item-danger"
            onSelect={() => handleBlockUser(key, name)}
          >
            Block
          </ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    );
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
                onSelect={handleSelectFriend}
                renderItem={(friend, item) => (
                  <ContextMenu>
                    <ContextMenu.Trigger as="div">{item}</ContextMenu.Trigger>
                    {buddyMenu(friend)}
                  </ContextMenu>
                )}
              />
            )}
          </For>
        </Show>
      </Show>
      <SimpleInputModal
        isOpen={newGroupTarget() !== null}
        title="New group"
        placeholder="Group name"
        submitLabel="Create"
        onClose={() => setNewGroupTarget(null)}
        onSubmit={async (value) => {
          const target = newGroupTarget();
          if (target) {
            await createGroupAndMove(target, value);
          }
        }}
      />
    </ScrollArea>
  );
};

export default BuddyList;
