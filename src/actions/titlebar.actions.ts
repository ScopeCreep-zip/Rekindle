import { getCurrentWindow } from "@tauri-apps/api/window";

/** A titlebar action on this window. */
function onThisWindow(act: (w: ReturnType<typeof getCurrentWindow>) => Promise<void>): () => void {
  return () => void act(getCurrentWindow());
}

export const handleMinimize = onThisWindow((w) => w.minimize());

/** Hide the window (keeps process alive for system tray). Used by buddy list. */
export const handleHide = onThisWindow((w) => w.hide());

/** Close the window (destroys webview). Used by chat, community, profile, settings. */
export const handleClose = onThisWindow((w) => w.close());

export const handleMaximize = onThisWindow((w) => w.toggleMaximize());
