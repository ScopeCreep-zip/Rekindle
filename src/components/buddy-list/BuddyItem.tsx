import { type Component, Show } from "solid-js";
import { handleRetryHandshake } from "../../actions/buddy.actions";
import { ICON_VOLUME_HIGH } from "../../icons";
import type { DeliveryRetry } from "../../ipc/channels/chat_events";
import type { UserStatus } from "../../stores/auth.store";
import { deliveryRetryText } from "../../utils/delivery_retry";
import { formatDuration } from "../../utils/formatting";
import { identityPinMismatchText } from "../../utils/identity_pin";
import { formatRelativeTime } from "../../utils/time";
import Tooltip from "../common/Tooltip";
import GameIcon from "../game/GameIcon";
import StatusDot from "../status/StatusDot";

interface BuddyItemProps {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: UserStatus;
  statusMessage: string | null;
  gameInfo: string | null;
  gameIconKey: string | null;
  gameElapsed: number | null;
  serverAddress: string | null;
  lastSeenAt: number | null;
  unreadCount: number;
  voiceChannel: string | null;
  friendshipState?: string;
  requestSendState?: string;
  /** Our request goes unanswered and the Host keeps trying (plan SR2). */
  requestRetry?: DeliveryRetry | null;
  handshakeError: string | null;
  handshakeRetrying: boolean;
  unreachable: boolean;
  identityPinMismatch: boolean;
  selected: boolean;
  /** The id of the label naming this row's section (Online or Offline),
   *  so the row read on its own says which (WCAG 1.3.1). */
  describedBy?: string;
  onDoubleClick: (publicKey: string, displayName: string) => void;
  onSelect: (publicKey: string) => void;
}

/** The header of the group `row` is listed under: the nearest level-1 node before it. */
function groupHeaderOf(row: HTMLElement): HTMLElement | null {
  const tree = row.closest('[role="tree"]');
  if (!tree) return null;
  const nodes = Array.from(tree.querySelectorAll<HTMLElement>('[role="treeitem"]'));
  const at = nodes.indexOf(row);
  for (let i = at - 1; i >= 0; i--) {
    if (nodes[i].getAttribute("aria-level") === "1") return nodes[i];
  }
  return null;
}

const BuddyItem: Component<BuddyItemProps> = (props) => {
  function handleDblClick(): void {
    props.onDoubleClick(props.publicKey, props.displayName);
  }

  function handleClick(): void {
    props.onSelect(props.publicKey);
  }

  // Plan E7.3: the row is a roving-tabindex tree node (`roving_list.ts`,
  // APG Treeview): focusing it selects it, as a click does, Enter opens the
  // chat, as a double click does, and ArrowLeft moves to its group's header.
  function handleKeyDown(e: KeyboardEvent): void {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter") {
      e.preventDefault();
      handleDblClick();
    } else if (e.key === "ArrowLeft") {
      const header = groupHeaderOf(e.currentTarget as HTMLElement);
      if (header) {
        e.preventDefault();
        header.focus();
      }
    }
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
      role="treeitem"
      tabIndex={-1}
      aria-level={2}
      aria-selected={props.selected}
      aria-describedby={props.describedBy}
      onClick={handleClick}
      onDblClick={handleDblClick}
      onFocus={handleClick}
      onKeyDown={handleKeyDown}
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
          <div class="buddy-game-info">
            <GameIcon iconKey={props.gameIconKey} />
            <span class="buddy-game-text">{gameDisplay()}</span>
          </div>
        </Show>
        <Show when={props.friendshipState === "pendingOut" && props.requestSendState}>
          {(state) => <div class="buddy-request-state">Request {state()}</div>}
        </Show>
        <Show when={props.friendshipState === "pendingOut" && props.requestRetry}>
          {(retry) => <div class="buddy-request-state">{deliveryRetryText(retry())}</div>}
        </Show>
        <Show when={!gameDisplay() && props.statusMessage}>
          <div class="buddy-status-message">{props.statusMessage}</div>
        </Show>
        <Show when={lastSeenText()}>
          <div class="buddy-last-seen">{lastSeenText()}</div>
        </Show>
        <Show when={props.identityPinMismatch}>
          <Tooltip text={identityPinMismatchText(props.displayName)}>
            <div class="buddy-identity-mismatch" role="alert">
              Identity key mismatch
            </div>
          </Tooltip>
        </Show>
        <Show when={props.unreachable}>
          <div class="buddy-handshake-retrying">Unreachable — messages wait in the outbox</div>
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
        <span class="buddy-voice-icon nf-icon" title="In voice channel">
          {ICON_VOLUME_HIGH}
        </span>
      </Show>
      <Show when={props.unreadCount > 0}>
        <span class="buddy-unread-badge">{props.unreadCount}</span>
      </Show>
    </div>
  );
};

export default BuddyItem;
