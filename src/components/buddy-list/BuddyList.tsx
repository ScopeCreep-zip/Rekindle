import { addToast } from "../../stores/toast.store";
import { type Component, createEffect, createMemo, createSignal, For, type JSX, Show } from "solid-js";
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
} from "../../actions/buddy.actions";
import {
  handleRevokeRelay,
  handleVolunteerRelay,
  relayState,
} from "../../actions/relay.actions";
import { handleStartDmCall } from "../../actions/calls.actions";
import { sendSkinSelection } from "../../actions/skin.actions";
import { handleLaunchGame } from "../../actions/game.actions";
import { notJoinableText } from "../../utils/joinable_text";
import { commands } from "../../ipc/commands";
import BuddyGroup from "./BuddyGroup";
import ScrollArea from "../common/ScrollArea";
import { rovingList } from "../../utils/roving_list";
import SimpleInputModal from "../common/SimpleInputModal";

/** The list of our sent requests, shown as a group of its own. */
const AWAITING_RESPONSE = "Awaiting Response";

function matchesSearch(friend: Friend, query: string): boolean {
  const q = query.toLowerCase();
  if (friend.displayName.toLowerCase().includes(q)) return true;
  if (friend.nickname && friend.nickname.toLowerCase().includes(q)) return true;
  return false;
}


/** Join the friend's game server through the Host's review; a refusal is shown. */
async function joinFriendGame(friend: Friend): Promise<void> {
  const game = friend.gameInfo;
  if (!game?.joinable || game.gameId === null || game.serverAddress === null) return;
  await handleLaunchGame(game.gameId, game.serverAddress, "friendPresence");
}

const BuddyList: Component = () => {
  const [selectedKey, setSelectedKey] = createSignal<string | null>(null);
  // A friend group's header selected instead (plan E7.15 IV5; null id = the default group).
  const [selectedGroup, setSelectedGroup] = createSignal<{ groupId: number | null } | null>(null);
  // A friend no longer listed (removed, blocked) is no longer selected.
  createEffect(() => {
    const key = selectedKey();
    if (key !== null && !(key in friendsState.friends)) setSelectedKey(null);
  });
  // The Host keeps this window's selection: the skin's selection-dependent
  // actions act on it and its capabilities come from it (plan SK2-A5).
  createEffect(() => {
    const key = selectedKey();
    const group = selectedGroup();
    void sendSkinSelection(
      key !== null
        ? { kind: "friend", arg: { publicKey: key } }
        : group !== null
          ? { kind: "friendGroup", arg: { groupId: group.groupId } }
          : null,
    );
  });
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
      const group = friend.group;
      if (!groups[group]) groups[group] = [];
      groups[group].push(friend);
    }
    const statusOrder = { online: 0, away: 1, busy: 2, offline: 3, invisible: 3 };
    for (const group of Object.values(groups)) {
      group.sort(
        (a, b) =>
          (statusOrder[a.status] ?? 3) - (statusOrder[b.status] ?? 3),
      );
    }
    if (pendingOut.length > 0) {
      groups[AWAITING_RESPONSE] = pendingOut;
    }
    return groups;
  });

  // The groups by name, so each group and its rows keep their elements
  // while the friends change (presence, a signal push on selection):
  // iterating `Object.entries` made new tuples on every change, `For`
  // re-created every group and row, and a double-click straddling a change
  // landed on a replaced row (plan CR1 audit).
  const groupNames = createMemo(() => Object.keys(groupedFriends()));

  const hasFriends = createMemo(() => Object.keys(friendsState.friends).length > 0);
  const hasFilteredResults = createMemo(() => Object.keys(groupedFriends()).length > 0);

  /** The id of the group a label names: ours, or null for the Host's
   *  default group and the pending-request list. */
  function groupIdOf(name: string): number | null {
    if (name === AWAITING_RESPONSE) return null;
    return groupedFriends()[name]?.[0]?.groupId ?? null;
  }

  /** Our groups a friend can move to (not the pending-request list); the
   *  Host's default group moves them out of any group (id null). */
  const existingGroups = createMemo(() =>
    Object.keys(groupedFriends()).filter((name) => name !== AWAITING_RESPONSE),
  );

  function handleDblClick(publicKey: string): void {
    setSelectedGroup(null);
    setSelectedKey(publicKey);
    handleDoubleClickFriend(publicKey);
  }

  function handleSelectFriend(publicKey: string): void {
    setSelectedGroup(null);
    setSelectedKey(publicKey);
  }

  function handleSelectGroup(groupId: number | null): void {
    setSelectedKey(null);
    setSelectedGroup({ groupId });
  }

  async function moveToExistingGroup(friendKey: string, groupName: string): Promise<void> {
    await handleMoveFriendToGroup(friendKey, groupIdOf(groupName));
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
              onSelect={() => handleBlockUser(key)}
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
            onSelect={() => commands.openChatWindow(key)}
          >
            Chat
          </ContextMenu.Item>
          {/* Wave 12 W12.8 — quick-call entries so the user doesn't have
           *  to open the chat window first. handleStartDmCall seeds the
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
            onSelect={() => commands.openProfileWindow(key)}
          >
            View Profile
          </ContextMenu.Item>

          {/* Always shown (owner: no button is hidden); enabled only while
           *  the Host states the friend's game is joinable (IV3c), and
           *  otherwise says why. A refused launch is shown, never dropped. */}
          <ContextMenu.Item
            class="context-menu-item"
            disabled={friend.gameInfo?.joinable !== true}
            onSelect={() => void joinFriendGame(friend)}
          >
            Join Game
            <Show when={friend.gameInfo?.joinable !== true}>
              <span class="context-menu-item-reason">{notJoinableText(friend.gameInfo)}</span>
            </Show>
          </ContextMenu.Item>

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
            onSelect={() =>
              void handleResetSignalSession(key).then((err) => {
                if (err) addToast(err, "error");
              })
            }
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
            onSelect={() => handleBlockUser(key)}
          >
            Block
          </ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    );
  }

  return (
    <ScrollArea class="buddy-list" ref={rovingList(".buddy-item")}>
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
          <For each={groupNames()}>
            {(name) => (
              <BuddyGroup
                name={name}
                groupId={groupIdOf(name)}
                friends={groupedFriends()[name] ?? []}
                selectedKey={selectedKey()}
                groupSelected={name !== AWAITING_RESPONSE && selectedGroup()?.groupId === groupIdOf(name) && selectedGroup() !== null}
                onSelectGroup={name === AWAITING_RESPONSE ? undefined : () => handleSelectGroup(groupIdOf(name))}
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
