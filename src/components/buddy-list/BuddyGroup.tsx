import { Component, createSignal, Show, For } from "solid-js";
import type { Friend } from "../../stores/friends.store";
import { handleRenameFriendGroup, handleCreateFriendGroup } from "../../handlers/buddy.handlers";
import BuddyItem from "./BuddyItem";

interface BuddyGroupProps {
  name: string;
  friends: Friend[];
  selectedKey: string | null;
  onDoubleClick: (publicKey: string, displayName: string) => void;
  onContextMenu: (e: MouseEvent, publicKey: string) => void;
}

const BuddyGroup: Component<BuddyGroupProps> = (props) => {
  const [collapsed, setCollapsed] = createSignal(false);
  const [renaming, setRenaming] = createSignal(false);
  const [renameValue, setRenameValue] = createSignal("");

  function handleToggle(): void {
    setCollapsed(!collapsed());
  }

  function handleGroupContextMenu(e: MouseEvent): void {
    e.preventDefault();
    // Don't allow renaming the default "Friends" group
    if (props.name === "Friends") return;
    setRenameValue(props.name);
    setRenaming(true);
  }

  async function commitRename(): Promise<void> {
    const newName = renameValue().trim();
    if (newName && newName !== props.name) {
      // Create/get the group ID then rename
      const groupId = await handleCreateFriendGroup(props.name);
      if (groupId >= 0) {
        await handleRenameFriendGroup(groupId, newName);
      }
    }
    setRenaming(false);
  }

  function handleRenameKeyDown(e: KeyboardEvent): void {
    if (e.key === "Enter") {
      commitRename();
    } else if (e.key === "Escape") {
      setRenaming(false);
    }
  }

  const isPendingGroup = () => props.name === "Awaiting Response";

  return (
    <div class={isPendingGroup() ? "buddy-group-pending" : ""}>
      <Show when={!renaming()} fallback={
        <div class="group-create-inline">
          <input
            class="group-create-input"
            type="text"
            value={renameValue()}
            onInput={(e) => setRenameValue(e.currentTarget.value)}
            onKeyDown={handleRenameKeyDown}
            onBlur={() => commitRename()}
            autofocus
          />
          <button
            class="group-create-ok-btn"
            onClick={() => commitRename()}
          >
            OK
          </button>
        </div>
      }>
        <div
          class="buddy-group-header"
          onClick={handleToggle}
          onContextMenu={handleGroupContextMenu}
        >
          {collapsed() ? "\u25B6" : "\u25BC"} {props.name} ({props.friends.length})
        </div>
      </Show>
      <Show when={!collapsed()}>
        <For each={props.friends}>
          {(friend) => (
            <BuddyItem
              publicKey={friend.publicKey}
              displayName={friend.displayName}
              nickname={friend.nickname}
              status={friend.status}
              statusMessage={friend.statusMessage}
              gameInfo={friend.gameInfo?.gameName ?? null}
              gameElapsed={friend.gameInfo?.startedAt ?? null}
              lastSeenAt={friend.lastSeenAt}
              unreadCount={friend.unreadCount}
              voiceChannel={friend.voiceChannel}
              friendshipState={friend.friendshipState}
              selected={props.selectedKey === friend.publicKey}
              onDoubleClick={props.onDoubleClick}
              onContextMenu={props.onContextMenu}
            />
          )}
        </For>
      </Show>
    </div>
  );
};

export default BuddyGroup;
