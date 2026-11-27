import { trackWindowFocus } from "./window_focus";

/**
 * The window kinds (plan SB1): the G11b `WindowLabel` variants, without ids.
 * The Host scopes each pack's art to the kinds the pack defines, on the
 * document root, so a window's portaled menus and dialogs (mounted under
 * `<body>`) are in scope too, and an unskinned window (Settings, …) is not.
 */
export type WindowKind =
  | "login"
  | "buddy-list"
  | "settings"
  | "skin-editor"
  | "chat"
  | "group-chat"
  | "community"
  | "community-browser"
  | "call"
  | "profile"
  | "popup";

/** Name this window's kind on its document root (`<html data-window>`), the one place the Host's sheet reads it;
 *  every window calls it, so it also starts the window's focus state (`window_focus.ts`). */
export function setWindowKind(kind: WindowKind): void {
  document.documentElement.dataset.window = kind;
  trackWindowFocus();
}
