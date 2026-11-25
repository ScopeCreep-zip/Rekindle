// The app menu's own operations (the buddy list's Rekindle menu, a skin's
// File, Tool and Help items): Settings on a tab, Quit, the docs and a bug
// report. Shared by the buddy list's MenuBar and the skin shell runners
// (shell_runners/menu.ts), so no action imports a component.
import { commands } from "../ipc/commands";
import type { SettingsTab } from "../ipc/commands/types";

const REPO_URL = "https://github.com/ScopeCreep-zip/Rekindle";

export function openSettingsTab(tab: SettingsTab): void {
  commands.openSettingsWindow(tab);
}

/** Quit the app, as the tray's Quit does; closing this window only hides
 *  it in the tray. */
export function handleQuit(): void {
  void commands.quitApp();
}

export function handleOpenDocs(): void {
  void commands.openExternalUrl(`${REPO_URL}/tree/main/docs`);
}

export function handleReportBug(): void {
  void commands.openExternalUrl(`${REPO_URL}/issues/new`);
}
