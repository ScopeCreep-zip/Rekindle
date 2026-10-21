import { Component, createMemo, createSignal, Show } from "solid-js";
import type { FriendCodeView } from "../../ipc/commands/dto";
import { qrPath, qrSize } from "../../utils/qr";

/**
 * Our friend code (plan E2.5, D17): the `rekindle://add/` code, its QR drawn
 * as a path from the Host's module rows, a copy button and the token
 * rotation. A security surface (plan E7.7.0): it sits on the fixed `--sec-*`
 * card, so no skin can hide or swap what a friend scans or copies.
 */
const FriendCodeCard: Component<{
  view: FriendCodeView;
  rotating: boolean;
  onRotate: () => void;
}> = (props) => {
  const [copied, setCopied] = createSignal(false);
  const size = createMemo(() => qrSize(props.view.qrRows));

  async function copy(): Promise<void> {
    await navigator.clipboard.writeText(props.view.url);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  return (
    <section class="sec-friend-code" aria-label="Your friend code">
      <svg
        class="sec-friend-code-qr"
        viewBox={`0 0 ${size()} ${size()}`}
        shape-rendering="crispEdges"
        role="img"
        aria-label="Your friend code as a QR code"
      >
        <rect class="sec-friend-code-qr-ground" width={size()} height={size()} />
        <path class="sec-friend-code-qr-modules" d={qrPath(props.view.qrRows)} />
      </svg>
      <output class="sec-friend-code-url" aria-label="Your friend code">
        {props.view.url}
      </output>
      <div class="sec-friend-code-actions">
        <button type="button" class="sec-btn-secondary" onClick={() => void copy()}>
          <Show when={copied()} fallback="Copy code">
            Copied!
          </Show>
        </button>
        <button
          type="button"
          class="sec-btn-secondary"
          disabled={props.rotating}
          aria-describedby="friend-code-rotate-hint"
          onClick={() => props.onRotate()}
        >
          New code
        </button>
      </div>
      <p id="friend-code-rotate-hint" class="sec-friend-code-hint">
        A new code stops the old one from working. Your friends stay your friends.
      </p>
    </section>
  );
};

export default FriendCodeCard;
