import { Accessor, createSignal, onCleanup, onMount } from "solid-js";
import { commands } from "../ipc/commands";
import { subscribeChatEvents } from "../ipc/channels";
import type { SafetyNumberView } from "../ipc/commands/dto";
import { errorMessage } from "../utils/error";

/**
 * A peer's safety number and verified state for this window (plan E2.3),
 * read from the Host on mount and on `reload` after this window's own mark
 * or clear, and updated from the Host's `verificationChanged` (any window's
 * mark or clear) and `keyChanged` (their identity rotated) events. `null`
 * until the first read.
 */
export function useSafetyNumber(peerKey: string): {
  view: Accessor<SafetyNumberView | null>;
  reload: () => void;
} {
  const [view, setView] = createSignal<SafetyNumberView | null>(null);
  const reload = (): void => {
    commands
      .getSafetyNumber(peerKey)
      .then(setView)
      .catch((e) => console.error("Safety number not read:", errorMessage(e)));
  };
  const unsubscribe = subscribeChatEvents((event) => {
    if (!("friend" in event)) return;
    const friend = event.friend;
    if ("verificationChanged" in friend && friend.verificationChanged.peerKey === peerKey) {
      reload();
    } else if ("keyChanged" in friend && friend.keyChanged.peerKey === peerKey) {
      reload();
    }
  });
  onMount(reload);
  onCleanup(() => void unsubscribe.then((un) => un()));
  return { view, reload };
}
