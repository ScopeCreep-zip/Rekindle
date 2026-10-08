import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeDeepLinkEvents } from "../ipc/channels";
import { commands } from "../ipc/commands";
import { setPendingDeepLink } from "../stores/deep-link.store";

/// Pull the deep link held for consent (one received before login, or
/// while this window was not listening).
export async function loadPendingDeepLink(): Promise<void> {
  setPendingDeepLink(await commands.getPendingDeepLink());
}

/// Show each new deep link in the consent dialog. Nothing is joined or
/// added until the user confirms it there.
export function subscribeDeepLinkHandler(): Promise<UnlistenFn> {
  return subscribeDeepLinkEvents((request) => setPendingDeepLink(request));
}
