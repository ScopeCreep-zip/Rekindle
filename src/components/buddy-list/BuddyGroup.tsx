import { Component, createSignal, JSX, Show, For } from "solid-js";
import type { Friend } from "../../stores/friends.store";
import { handleRenameFriendGroup, handleCreateFriendGroup } from "../../handlers/buddy.handlers";
import BuddyItem from "./BuddyItem";

interface BuddyGroupProps {
  name: string;
  friends: Friend[];
  selectedKey: string | null;
  onDoubleClick: (publicKey: string, displayName: string) => void;
  onSelect: (publicKey: string) => void;
  /**
   * Per-item render hook owned by the parent so context-menu wrapping
   * (`<ContextMenu><ContextMenu.Trigger>...</>`) lives at the level
   * where the menu's data dependencies (auth, relay state, friend
   * groups, etc.) are in scope.
   */
  renderItem: (friend: Friend, item: JSX.Element) => JSX.Element;
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
    if (props.name === "Friends") return;
    setRenameValue(props.name);
    setRenaming(true);
  }

  async function commitRename(): Promise<void> {
    const newName = renameValue().trim();
    if (newName && newName !== props.name) {
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
            class="form-btn-primary"
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
          {collapsed() ? "▶" : "▼"} {props.name} ({props.friends.length})
        </div>
      </Show>
      <Show when={!collapsed()}>
        <For each={props.friends}>
          {(friend) =>
            props.renderItem(
              friend,
              <BuddyItem
                publicKey={friend.publicKey}
                displayName={friend.displayName}
                nickname={friend.nickname}
                status={friend.status}
                statusMessage={friend.statusMessage}
                gameInfo={friend.gameInfo?.gameName ?? null}
                gameElapsed={friend.gameInfo?.startedAt ?? null}
                serverAddress={friend.gameInfo?.serverAddress ?? null}
                lastSeenAt={friend.lastSeenAt}
                unreadCount={friend.unreadCount}
                voiceChannel={friend.voiceChannel}
                friendshipState={friend.friendshipState}
                selected={props.selectedKey === friend.publicKey}
                onDoubleClick={props.onDoubleClick}
                onSelect={props.onSelect}
              />,
            )
          }
        </For>
      </Show>
    </div>
  );
};

export default BuddyGroup;
