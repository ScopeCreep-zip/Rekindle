import { type Component, createMemo, createSignal, onCleanup, Show } from "solid-js";
import { remainingShort, remainingText } from "../../utils/disappearing";
import SkinGlyph from "../common/SkinGlyph";

// One clock for every indicator on the page, running only while one is
// shown. It only redraws the countdown: the Host deletes the message at
// its `expireAt` and the window drops it on `messagesExpired`.
const [now, setNow] = createSignal(Date.now());
let shown = 0;
let tick: ReturnType<typeof setInterval> | undefined;

function watchClock(): void {
  shown += 1;
  if (tick === undefined) {
    setNow(Date.now());
    tick = setInterval(() => setNow(Date.now()), 1000);
  }
}

function unwatchClock(): void {
  shown -= 1;
  if (shown === 0 && tick !== undefined) {
    clearInterval(tick);
    tick = undefined;
  }
}

/** A message's disappearing countdown (plan E7.1): the time left until the
 *  Host's `expireAt`, or a still glyph while a received message's timer
 *  waits for it to be displayed. */
const ExpiryIndicator: Component<{ expireAt?: number; expireSeconds?: number }> = (props) => {
  watchClock();
  onCleanup(unwatchClock);
  const countdown = createMemo(() => {
    const expireAt = props.expireAt;
    return expireAt === undefined ? undefined : { ms: expireAt - now() };
  });
  return (
    <Show
      when={countdown()}
      fallback={
        <Show when={(props.expireSeconds ?? 0) > 0}>
          <span class="message-expiry" title="Disappears once read">
            <SkinGlyph name="timer-sand" />
            <span class="sr-only">Disappears once read</span>
          </span>
        </Show>
      }
    >
      {(remaining) => (
        <span class="message-expiry" title={`Disappears in ${remainingText(remaining().ms)}`}>
          <SkinGlyph name="timer-sand" />
          <span aria-hidden="true">{remainingShort(remaining().ms)}</span>
          <span class="sr-only">Disappears in {remainingText(remaining().ms)}</span>
        </span>
      )}
    </Show>
  );
};

export default ExpiryIndicator;
