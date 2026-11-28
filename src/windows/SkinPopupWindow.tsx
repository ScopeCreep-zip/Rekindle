import { type Component, onCleanup, onMount, Show } from "solid-js";
import { callOps, chatOpenOps, friendOps, menuOps, transferOps } from "../actions/shell_runners";
import { registerShellRunner } from "../actions/skin.actions";
import { handleClose } from "../actions/titlebar.actions";
import Infoview from "../components/infoview/Infoview";
import SkinTreeWindow from "../components/skin-tree/SkinTreeWindow";
import { popupName, skinPopupShell } from "../hooks/useSkin";
import { startEventStream, subscribeSkinSignals, type UnlistenFn } from "../ipc/channels";
import { setSkinSignals } from "../stores/skin-signals.store";
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
        })
      )
    );
    // The groups the pack popups' trees serve (shell_runners): the menus
    // a main-menu popup carries, a friend's profile and chat, downloads,
    // and a call started or ended. No mute: no popup lays one. A popup's
    // LaunchPopup is not run here: `open_skin_popup` takes only a skin
    // window as launcher, so it says it has no handler in this window.
    for (const run of [menuOps, friendOps, chatOpenOps, transferOps, callOps]) {
      unlisteners.push(Promise.resolve(registerShellRunner(run)));
    }
    void startEventStream();
  });
  onCleanup(() => {
    for (const p of unlisteners) void p.then((unlisten) => unlisten());
  });
  setWindowKind("popup");
  return (
    <div class="app-frame" data-skin-popup={name}>
      <Show when={skinPopupShell(name)} keyed>
        {(shell) => (
          <SkinTreeWindow
            shell={shell}
            views={() => null}
            communities={() => null}
            pieces={{ infoView: () => <Infoview subject={{ type: "selection" }} /> }}
          />
        )}
      </Show>
    </div>
  );
};

export default SkinPopupWindow;
