import { type Component, createSignal, createUniqueId, For, type JSX, Show } from "solid-js";
import { handleRenameFriendGroup } from "../../actions/buddy.actions";
import type { Friend } from "../../stores/friends.store";
import BuddyItem from "./BuddyItem";

interface BuddyGroupProps {
  name: string;
  /** The id of our group (the Host's `friend_groups.id`), or null for the
   *  default group and the request list, which cannot be renamed. */
  groupId: number | null;
  friends: Friend[];
  selectedKey: string | null;
  /** This group's header is the window's selection (plan E7.15 IV5). */
  groupSelected?: boolean;
  /** Selecting the header makes the group the selection; absent for the request list. */
  onSelectGroup?: () => void;
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

  /** Clicking the header selects the group and folds or unfolds it. */
  function activateHeader(): void {
    props.onSelectGroup?.();
    handleToggle();
  }

  // The header is a tree node (APG Treeview): Enter and Space act as a
  // click, ArrowRight unfolds a folded group and ArrowLeft folds an open
  // one; ArrowUp/ArrowDown, Home and End move between rows (roving_list).
  function handleHeaderKeyDown(e: KeyboardEvent): void {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      activateHeader();
    } else if (e.key === "ArrowRight" && collapsed()) {
      e.preventDefault();
      setCollapsed(false);
    } else if (e.key === "ArrowLeft" && !collapsed()) {
      e.preventDefault();
      setCollapsed(true);
    }
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
  // Xfire's list did; the list keeps its status order inside each. One
  // keyed list holds both, each section's label drawn on its first row: a
  // friend whose presence changes moves between the sections with the same
  // element (two lists re-created the row, and a double-click straddling a
  // presence change opened nothing, plan CR1 audit).
  const online = () => props.friends.filter((f) => f.status !== "offline");
  const offline = () => props.friends.filter((f) => f.status === "offline");
  const ordered = () => [...online(), ...offline()];
  const sectionOf = (f: Friend): "Online" | "Offline" =>
    f.status === "offline" ? "Offline" : "Online";
  const section = (f: Friend): Friend[] => (sectionOf(f) === "Online" ? online() : offline());
  // Every row is described by its section's label (WCAG 1.3.1: the meaning
  // the per-section groups gave, kept with one list); the label is not a
  // heading, since a tree's children are its nodes.
  const sectionIds = { Online: createUniqueId(), Offline: createUniqueId() };

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
        requestRetry={friend.requestRetry}
        handshakeError={friend.handshakeError}
        handshakeRetrying={friend.handshakeRetrying}
        unreachable={friend.unreachable}
        identityPinMismatch={friend.identityPinMismatch}
        selected={props.selectedKey === friend.publicKey}
        describedBy={isPendingGroup() ? undefined : sectionIds[sectionOf(friend)]}
        onDoubleClick={props.onDoubleClick}
        onSelect={props.onSelect}
      />
    );

  return (
    <div class={isPendingGroup() ? "buddy-group-pending" : ""}>
      <Show
        when={!renaming()}
        fallback={
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
            <button type="button" class="form-btn-primary" onClick={() => commitRename()}>
              OK
            </button>
          </div>
        }
      >
        <div
          class="buddy-group-header"
          classList={{ "buddy-group-header-selected": !!props.groupSelected }}
          role="treeitem"
          tabIndex={-1}
          aria-level={1}
          aria-expanded={!collapsed()}
          aria-selected={!!props.groupSelected}
          onClick={activateHeader}
          onKeyDown={handleHeaderKeyDown}
          onContextMenu={handleGroupContextMenu}
        >
          <span aria-hidden="true">{collapsed() ? "▶" : "▼"}</span>
          <span class="buddy-group-name">{props.name}</span>
          <span class="buddy-group-count">({props.friends.length})</span>
        </div>
      </Show>
      <Show when={!collapsed()}>
        <Show
          when={isPendingGroup()}
          fallback={
            <For each={ordered()}>
              {(friend) => (
                <div class="buddy-subgroup" data-buddy-section={sectionOf(friend)}>
                  <Show when={section(friend)[0] === friend}>
                    <div class="buddy-subgroup-header" id={sectionIds[sectionOf(friend)]}>
                      {sectionOf(friend)} ({section(friend).length})
                    </div>
                  </Show>
                  {renderBuddy(friend)}
                </div>
              )}
            </For>
          }
        >
          <For each={props.friends}>{renderBuddy}</For>
        </Show>
      </Show>
    </div>
  );
};

export default BuddyGroup;
