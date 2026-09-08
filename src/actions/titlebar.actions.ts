import { getCurrentWindow } from "@tauri-apps/api/window";

export function handleMinimize(): void {
  getCurrentWindow().minimize();
}

/** Hide the window (keeps process alive for system tray). Used by buddy list. */
export function handleHide(): void {
  getCurrentWindow().hide();
}

/** Close the window (destroys webview). Used by chat, community, profile, settings. */
export function handleClose(): void {
  getCurrentWindow().close();
}

export function handleMaximize(): void {
  getCurrentWindow().toggleMaximize();
}
