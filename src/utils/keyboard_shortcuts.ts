/**
 * The keyboard shortcuts the app has, as the help dialog lists them
 * (wiring guide §13f). Each row names where the handler lives, so a
 * changed key is changed here too, and the global chords match
 * `src-tauri/src/shortcuts.rs`.
 *
 * `Mod` is Cmd on macOS and Ctrl elsewhere, as the handlers read it.
 */
export interface ShortcutRow {
  keys: string[][];
  action: string;
}

export interface ShortcutSection {
  title: string;
  rows: ShortcutRow[];
}

export const SHORTCUT_SECTIONS: ShortcutSection[] = [
  {
    // src-tauri/src/shortcuts.rs CHORDS: registered with the OS, so they
    // work while another program has focus.
    title: "Anywhere",
    rows: [
      { keys: [["Mod", "Shift", "X"]], action: "Show or hide the buddy list" },
      { keys: [["Mod", "Shift", "M"]], action: "Mute or unmute yourself in a call" },
    ],
  },
  {
    // src/windows/BuddyListWindow.tsx handleKeyboardShortcuts.
    title: "Buddy list",
    rows: [
      { keys: [["Alt", "1"]], action: "Friends tab" },
      { keys: [["Alt", "2"]], action: "Communities tab" },
      { keys: [["Alt", "3"]], action: "Servers tab" },
      { keys: [["Alt", "4"]], action: "Downloads tab" },
      { keys: [["Mod", "F"]], action: "Search friends" },
    ],
  },
  {
    // src/windows/community_window/useCommunityWindow.ts.
    title: "Community",
    rows: [{ keys: [["Mod", "F"], ["Mod", "K"]], action: "Search messages" }],
  },
  {
    // src/utils/roving_list.ts: buddy, channel and member lists.
    title: "Lists",
    rows: [
      { keys: [["↑"], ["↓"]], action: "Move through the list" },
      { keys: [["Home"], ["End"]], action: "First or last item" },
      { keys: [["Enter"]], action: "Open the selected item" },
      { keys: [["Shift", "F10"], ["Menu"]], action: "Open the item's menu" },
    ],
  },
  {
    // src/components/chat/MessageInput.tsx.
    title: "Chat",
    rows: [
      { keys: [["Enter"]], action: "Send the message" },
      { keys: [["Shift", "Enter"]], action: "New line" },
    ],
  },
  {
    // Kobalte modal dialogs (wiring guide §13c) and this dialog.
    title: "Dialogs",
    rows: [
      { keys: [["Tab"], ["Shift", "Tab"]], action: "Move between controls" },
      { keys: [["Esc"]], action: "Close the dialog" },
      { keys: [["Mod", "/"]], action: "Show these shortcuts" },
    ],
  },
];

/** True on macOS, where `Mod` is Cmd. */
export function isMac(): boolean {
  return /Mac/i.test(navigator.platform);
}

/** A key's label on this platform. */
export function keyLabel(key: string): string {
  if (key !== "Mod") return key;
  return isMac() ? "Cmd" : "Ctrl";
}

/** The chord that opens the help dialog: `Mod` + `/`. */
export function isHelpChord(e: KeyboardEvent): boolean {
  return (isMac() ? e.metaKey : e.ctrlKey) && !e.altKey && e.key === "/";
}
