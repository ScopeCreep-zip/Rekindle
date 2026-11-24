import { Component, Show, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import SkinTreeWindow from "../components/skin-tree/SkinTreeWindow";
import { popupName, skinPopupShell } from "../hooks/useSkin";
import { startEventStream, subscribeSkinSignals } from "../ipc/channels";
import { setSkinSignals } from "../stores/skin-signals.store";
import { registerShellRunner } from "../actions/skin.actions";
import { handleClose } from "../actions/titlebar.actions";
import { setWindowKind } from "../utils/window_kind";

/**
 * A pack popup (plan SK2-P1b; `LaunchSkin`'s `<Popup>`: Xfire's Infoview
 * beside the buddy list, StealthX's main menu) in its own window, opened by
 * the shell only after the Host answered the pack's `LaunchPopup`. Its tree
 * is the served popup shell, its frame the skin's `data-skin-popup` rules;
 * its triggers run through the same dispatcher as every skin window.
 */
const SkinPopupWindow: Component = () => {
  const name = popupName();
  const unlisteners: Promise<UnlistenFn>[] = [];
  onMount(() => {
    unlisteners.push(subscribeSkinSignals(setSkinSignals));
    // Close shuts the popup (its opener stays).
    unlisteners.push(
      Promise.resolve(
        registerShellRunner((op) => {
          if (op.kind !== "close") return false;
          handleClose();
          return true;
        }),
      ),
    );
    void startEventStream();
  });
  onCleanup(() => {
    for (const p of unlisteners) void p.then((unlisten) => unlisten());
  });
  setWindowKind("popup");
  return (
    <div class="app-frame" data-skin-popup={name}>
      <Show when={skinPopupShell(name)} keyed>
        {(shell) => <SkinTreeWindow shell={shell} views={() => null} communities={() => null} />}
      </Show>
    </div>
  );
};

export default SkinPopupWindow;
