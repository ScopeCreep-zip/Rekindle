import { createSignal } from "solid-js";

/** Whether this window's keyboard-shortcuts dialog is open (§13f). */
const [shortcutsHelpOpen, setShortcutsHelpOpen] = createSignal(false);

export { setShortcutsHelpOpen, shortcutsHelpOpen };
