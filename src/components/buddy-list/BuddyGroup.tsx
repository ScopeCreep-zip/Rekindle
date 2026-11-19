import { Component, createSignal, JSX, Show, For } from "solid-js";
import type { Friend } from "../../stores/friends.store";
import { handleRenameFriendGroup } from "../../actions/buddy.actions";
import BuddyItem from "./BuddyItem";

interface BuddyGroupProps {
  name: string;
  /** The id of our group (the Host's `friend_groups.id`), or null for the
   *  default group and the request list, which cannot be renamed. */
  groupId: number | null;
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

/** One status section of a group; an empty one is not shown. */
const BuddySection: Component<{
  label: string;
  friends: Friend[];
  render: (friend: Friend) => JSX.Element;
}> = (props) => (
  <Show when={props.friends.length > 0}>
    <div class="buddy-subgroup" role="group" aria-label={props.label}>
      <div class="buddy-subgroup-header">
        {props.label} ({props.friends.length})
      </div>
      <For each={props.friends}>{props.render}</For>
    </div>
  </Show>
);

const BuddyGroup: Component<BuddyGroupProps> = (props) => {
  const [collapsed, setCollapsed] = createSignal(false);
  const [renaming, setRenaming] = createSignal(false);
  const [renameValue, setRenameValue] = createSignal("");

  function handleToggle(): void {
    setCollapsed(!collapsed());
  }

  function handleGroupContextMenu(e: MouseEvent): void {
    e.preventDefault();
    if (props.groupId === null) return;
    setRenameValue(props.name);
    setRenaming(true);
  }

  async function commitRename(): Promise<void> {
    // Enter commits, and the input's removal then blurs it: leave rename
    // mode first so the blur does not send the rename a second time.
    if (!renaming()) return;
    setRenaming(false);
    const newName = renameValue().trim();
    const groupId = props.groupId;
    if (newName && newName !== props.name && groupId !== null) {
      await handleRenameFriendGroup(groupId, newName);
    }
  }

  function handleRenameKeyDown(e: KeyboardEvent): void {
    if (e.key === "Enter") {
      commitRename();
    } else if (e.key === "Escape") {
      setRenaming(false);
    }
  }

  const isPendingGroup = () => props.name === "Awaiting Response";

  // Plan E7.2: each group splits into "Online" and "Offline" sections, as
  // Xfire's list did; the list keeps its status order inside each.
  const online = () => props.friends.filter((f) => f.status !== "offline");
  const offline = () => props.friends.filter((f) => f.status === "offline");

  const renderBuddy = (friend: Friend) =>
    props.renderItem(
      friend,
      <BuddyItem
        publicKey={friend.publicKey}
        displayName={friend.displayName}
        nickname={friend.nickname}
        status={friend.status}
        statusMessage={friend.statusMessage}
        gameInfo={friend.gameInfo?.gameName ?? null}
        gameIconKey={friend.gameInfo?.iconKey ?? null}
        gameElapsed={friend.gameInfo?.startedAt ?? null}
        serverAddress={friend.gameInfo?.serverAddress ?? null}
        lastSeenAt={friend.lastSeenAt}
        unreadCount={friend.unreadCount}
        voiceChannel={friend.voiceChannel}
        friendshipState={friend.friendshipState}
        requestSendState={friend.requestSendState}
        handshakeError={friend.handshakeError}
        handshakeRetrying={friend.handshakeRetrying}
        unreachable={friend.unreachable}
        identityPinMismatch={friend.identityPinMismatch}
        selected={props.selectedKey === friend.publicKey}
        onDoubleClick={props.onDoubleClick}
        onSelect={props.onSelect}
      />,
    );

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
          <span aria-hidden="true">{collapsed() ? "▶" : "▼"}</span>
          <span class="buddy-group-name">{props.name}</span>
          <span class="buddy-group-count">({props.friends.length})</span>
        </div>
      </Show>
      <Show when={!collapsed()}>
        <Show when={isPendingGroup()} fallback={
          <>
            <BuddySection label="Online" friends={online()} render={renderBuddy} />
            <BuddySection label="Offline" friends={offline()} render={renderBuddy} />
          </>
        }>
          <For each={props.friends}>{renderBuddy}</For>
        </Show>
      </Show>
    </div>
  );
};

export default BuddyGroup;
