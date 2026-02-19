import { Component, Show } from "solid-js";
import StatusDot from "../status/StatusDot";
import Tooltip from "../common/Tooltip";
import type { UserStatus } from "../../stores/auth.store";
import { ICON_VOLUME_HIGH } from "../../icons";

interface BuddyItemProps {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: UserStatus;
  statusMessage: string | null;
  gameInfo: string | null;
  gameElapsed: number | null;
  lastSeenAt: number | null;
  unreadCount: number;
  voiceChannel: string | null;
  friendshipState?: string;
  selected: boolean;
  onDoubleClick: (publicKey: string, displayName: string) => void;
  onContextMenu: (e: MouseEvent, publicKey: string) => void;
}

function formatElapsed(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  if (hours > 0) {
    return `${hours}h ${mins}m`;
  }
  return `${mins}m`;
}

function formatRelativeTime(timestampMs: number): string {
  const now = Date.now();
  const diffMs = now - timestampMs;
  const diffSec = Math.floor(diffMs / 1000);

  if (diffSec < 60) return "just now";

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;

  const diffHours = Math.floor(diffMin / 60);
  if (diffHours < 24) return `${diffHours}h ago`;

  const diffDays = Math.floor(diffHours / 24);
  if (diffDays < 30) return `${diffDays}d ago`;

  const diffMonths = Math.floor(diffDays / 30);
  return `${diffMonths}mo ago`;
}

const BuddyItem: Component<BuddyItemProps> = (props) => {
  function handleDblClick(): void {
    props.onDoubleClick(props.publicKey, props.displayName);
  }

  function handleCtxMenu(e: MouseEvent): void {
    props.onContextMenu(e, props.publicKey);
  }

  const isOffline = () => props.status === "offline";

  const lastSeenText = () => {
    if (!isOffline() || !props.lastSeenAt) return null;
    return `Last seen ${formatRelativeTime(props.lastSeenAt)}`;
  };

  const gameDisplay = () => {
    if (!props.gameInfo) return null;
    if (props.gameElapsed && props.gameElapsed > 0) {
      return `${props.gameInfo} (${formatElapsed(props.gameElapsed)})`;
    }
    return props.gameInfo;
  };

  const tooltipText = () =>
    props.nickname ? `${props.displayName} (${props.nickname})` : props.displayName;

  return (
    <div
      class={`buddy-item ${props.selected ? "buddy-item-selected" : ""}`}
      onDblClick={handleDblClick}
      onContextMenu={handleCtxMenu}
    >
      <Show when={(props.friendshipState ?? "accepted") !== "pendingOut"}>
        <StatusDot status={props.status} />
      </Show>
      <div class="buddy-item-content">
        <Tooltip text={tooltipText()}>
          <div class={`buddy-name ${isOffline() ? "buddy-name-offline" : ""}`}>
            {props.displayName}
            <Show when={props.nickname}>
              <span class="buddy-nickname"> ({props.nickname})</span>
            </Show>
          </div>
        </Tooltip>
        <Show when={gameDisplay()}>
          <div class="buddy-game-info">{gameDisplay()}</div>
        </Show>
        <Show when={!gameDisplay() && props.statusMessage}>
          <div class="buddy-status-message">{props.statusMessage}</div>
        </Show>
        <Show when={lastSeenText()}>
          <div class="buddy-last-seen">{lastSeenText()}</div>
        </Show>
      </div>
      <Show when={props.voiceChannel}>
        <span class="buddy-voice-icon nf-icon" title="In voice channel">{ICON_VOLUME_HIGH}</span>
      </Show>
      <Show when={props.unreadCount > 0}>
        <span class="buddy-unread-badge">{props.unreadCount}</span>
      </Show>
    </div>
  );
};

export default BuddyItem;
