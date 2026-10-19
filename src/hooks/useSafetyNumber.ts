import { Accessor, createSignal, onCleanup, onMount } from "solid-js";
import { commands } from "../ipc/commands";
import type { SafetyNumberView } from "../ipc/commands/dto";
import { errorMessage } from "../utils/error";

/**
 * A peer's safety number and verified state for this window (plan E2.3),
 * read from the Host on mount, again whenever the window regains focus (a
 * profile or chat window may have marked them meanwhile), and on `reload`
 * after this window's own mark or clear. `null` until the first read.
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
  onMount(() => {
    reload();
    window.addEventListener("focus", reload);
  });
  onCleanup(() => window.removeEventListener("focus", reload));
  return { view, reload };
}
