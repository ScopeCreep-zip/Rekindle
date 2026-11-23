import { onCleanup, onMount } from "solid-js";
import { commands } from "../ipc/commands";
import { isAppearanceChord } from "../utils/keyboard_shortcuts";

/**
 * Mod+, opens Settings at Appearance from this window (plan SK14): the skin
 * picker, with the accessible skin, is one keyboard step from every skin
 * whatever its menus offer. The login window has no identity to open
 * Settings for, so it does not listen.
 */
export function useAppearanceChord(route: () => string): void {
  const onKeyDown = (e: KeyboardEvent): void => {
    if (!isAppearanceChord(e) || route() === "/login") return;
    e.preventDefault();
    void commands.openSettingsWindow("appearance");
  };
  onMount(() => window.addEventListener("keydown", onKeyDown));
  onCleanup(() => window.removeEventListener("keydown", onKeyDown));
}
