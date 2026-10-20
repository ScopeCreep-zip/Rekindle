import { Component, Show } from "solid-js";
import { formatDuration } from "../../utils/formatting";
import StatusDot from "../status/StatusDot";
import Tooltip from "../common/Tooltip";
import type { UserStatus } from "../../stores/auth.store";
import { ICON_VOLUME_HIGH } from "../../icons";
import { formatRelativeTime } from "../../utils/time";
import { handleRetryHandshake } from "../../actions/buddy.actions";

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
  handshakeError: string | null;
  handshakeRetrying: boolean;
  selected: boolean;
  onDoubleClick: (publicKey: string, displayName: string) => void;
  onSelect: (publicKey: string) => void;
}

const BuddyItem: Component<BuddyItemProps> = (props) => {
  function handleDblClick(): void {
    props.onDoubleClick(props.publicKey, props.displayName);
  }

  function handleClick(): void {
    props.onSelect(props.publicKey);
  }

  // B7: the secure session did not open; Retry asks the Host to send the
  // first contact again (its review and the row's state come from there).
  function handleRetry(e: MouseEvent): void {
    e.stopPropagation();
    void handleRetryHandshake(props.publicKey);
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
      text += ` (${formatDuration(props.gameElapsed)})`;
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
        <Show when={props.handshakeRetrying && !props.handshakeError}>
          <div class="buddy-handshake-retrying">Retrying with a fresh key</div>
        </Show>
        <Show when={props.handshakeError}>
          <div class="buddy-handshake-failed">
            <Tooltip text={props.handshakeError ?? ""}>
              <div class="buddy-handshake-error">Secure session not set up</div>
            </Tooltip>
            <button type="button" class="pending-btn-accept" onClick={handleRetry}>
              Retry
            </button>
          </div>
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
