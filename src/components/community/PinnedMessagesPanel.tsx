import { type Component, For, Show, createMemo } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import type { Message } from "../../stores/chat.store";
import MessageBubble from "../chat/MessageBubble";
import { formatDateFromSecs } from "../../utils/formatting";

interface PinInfo {
  messageId: string;
  channelId: string;
  pinnedBy: string;
  pinnedAt: number;
}

interface PinnedMessagesPanelProps {
  pins: PinInfo[];
  messages: Message[];
  onClose: () => void;
  onUnpin: (messageId: string) => void;
  onJumpToMessage?: (messageId: string) => void;
}

const PinnedMessagesPanel: Component<PinnedMessagesPanelProps> = (props) => {
  const pinnedMessages = createMemo(() => {
    return props.pins.map((pin) => {
      const msg = props.messages.find((m) => m.serverMessageId === pin.messageId);
      return { ...pin, message: msg };
    }).filter((p) => p.message);
  });

  return (
    <div class="pin-panel">
      <div class="pin-panel-header">
        <SkinGlyph name="pin" />
        Pinned Messages ({pinnedMessages().length})
        <button class="modal-close-btn" onClick={props.onClose} aria-label="Close pinned messages panel">
          <SkinGlyph name="close" />
        </button>
      </div>
      <div class="pin-panel-list">
        <Show when={pinnedMessages().length === 0}>
          <div class="pin-panel-empty">No pinned messages</div>
        </Show>
        <For each={pinnedMessages()}>
          {(pin) => (
            <div class="pin-panel-item">
              <MessageBubble
                message={pin.message!}
                senderName={pin.message!.isOwn ? "You" : pin.message!.senderId}
              />
              <div class="pin-panel-item-meta">
                Pinned {formatDateFromSecs(pin.pinnedAt)}
                <span class="pin-panel-item-actions">
                  <Show when={props.onJumpToMessage}>
                    <button class="pin-panel-jump-btn" onClick={() => props.onJumpToMessage!(pin.messageId)}>Jump</button>
                  </Show>
                  <button class="pin-panel-unpin-btn" onClick={() => props.onUnpin(pin.messageId)}>Unpin</button>
                </span>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default PinnedMessagesPanel;
