import { type Component, For, createEffect, createSignal } from "solid-js";
import type { Message } from "../../stores/chat.store";
import { timerNoticeText } from "../../utils/disappearing";
import { messageAnnouncement } from "../../utils/message_announcement";

/** Lines kept in the region; older ones are dropped silently. */
const KEPT_LINES = 20;

interface ChatAnnouncerProps {
  messages: Message[];
  /**
   * False until the conversation's stored history has loaded. What is in
   * the list when it turns true is history and is not announced.
   */
  historyReady: boolean;
  senderName: (message: Message) => string;
}

/**
 * The chat window's new-message announcer (wiring guide §13b, WCAG 2.2
 * SC 4.1.3): a polite live region, rendered from mount, that reads each
 * message the peer sends while the window is open, without moving focus.
 *
 * It announces only messages appended after the last one it has seen.
 * History (the first load and every older page) and our own sends are not
 * announced: §13b's log is for status the user cannot see arrive, and
 * reading fifty stored messages, or the text the user just typed, would
 * drown the one new line. When the last seen message is gone (the list
 * was replaced), the current end becomes the new mark, again silently.
 */
const ChatAnnouncer: Component<ChatAnnouncerProps> = (props) => {
  const [lines, setLines] = createSignal<{ text: string }[]>([]);
  let mark: Message | undefined;
  let armed = false;

  createEffect(() => {
    const messages = props.messages;
    if (!props.historyReady) return;
    const last = messages[messages.length - 1];
    if (!armed) {
      armed = true;
      mark = last;
      return;
    }
    let fresh: Message[];
    if (mark === undefined) {
      // The conversation was empty: everything in it now is new.
      fresh = messages;
    } else {
      const at = messages.indexOf(mark);
      // Not found: the list was replaced, so its end is the new mark.
      fresh = at < 0 ? [] : messages.slice(at + 1);
    }
    mark = last;
    const added = fresh
      .filter((m) => !m.isOwn && m.historyGapThrough === undefined)
      .map((m) => ({
        text: m.timerNotice
          ? timerNoticeText(props.senderName(m), false, m.timerNotice.seconds)
          : messageAnnouncement(m, props.senderName(m), () => m.removedBy ?? ""),
      }));
    if (added.length > 0) setLines((prev) => [...prev, ...added].slice(-KEPT_LINES));
  });

  return (
    <div class="sr-only chat-announcer" aria-live="polite" aria-relevant="additions" aria-atomic="false">
      <For each={lines()}>{(line) => <p>{line.text}</p>}</For>
    </div>
  );
};

export default ChatAnnouncer;
