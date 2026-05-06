import { Component, Show } from "solid-js";
import StatusDot from "../status/StatusDot";
import Tooltip from "../common/Tooltip";
import type { UserStatus } from "../../stores/auth.store";
import { ICON_VOLUME_HIGH } from "../../icons";
import { formatRelativeTime } from "../../utils/time";

interface BuddyItemProps {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: UserStatus;
  statusMessage: string | null;
  gameInfo: string | null;
  gameElapsed: number | null;
  serverAddress: string | null;
  lastSeenAt: number | null;
  unreadCount: number;
  voiceChannel: string | null;
  friendshipState?: string;
  selected: boolean;
  onDoubleClick: (publicKey: string, displayName: string) => void;
  onSelect: (publicKey: string) => void;
}

function formatElapsed(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  if (hours > 0) {
    return `${hours}h ${mins}m`;
  }
  return `${mins}m`;
}

const BuddyItem: Component<BuddyItemProps> = (props) => {
  function handleDblClick(): void {
    props.onDoubleClick(props.publicKey, props.displayName);
  }

  function handleClick(): void {
    props.onSelect(props.publicKey);
  }

  const isOffline = () => props.status === "offline";

  const lastSeenText = () => {
    if (!isOffline() || !props.lastSeenAt) return null;
    return `Last seen ${formatRelativeTime(props.lastSeenAt)}`;
  };

  const gameDisplay = () => {
    if (!props.gameInfo) return null;
    let text = props.gameInfo;
    if (props.serverAddress) {
      text += ` on ${props.serverAddress}`;
    }
    if (props.gameElapsed && props.gameElapsed > 0) {
      text += ` (${formatElapsed(props.gameElapsed)})`;
    }
    return text;
  };

  const tooltipText = () =>
    props.nickname ? `${props.displayName} (${props.nickname})` : props.displayName;

  return (
    <div
      class={`buddy-item ${props.selected ? "buddy-item-selected" : ""}`}
      onClick={handleClick}
      onDblClick={handleDblClick}
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
