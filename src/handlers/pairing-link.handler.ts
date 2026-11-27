import { type UnlistenFn, subscribePairingLinkRefused } from "../ipc/channels";
import { setPairingLinkRefused } from "../stores/pairing-link.store";

/// Show the notice when the shell refuses a pairing link from the OS.
export function subscribePairingLinkHandler(): Promise<UnlistenFn> {
  return subscribePairingLinkRefused(() => setPairingLinkRefused(true));
}
