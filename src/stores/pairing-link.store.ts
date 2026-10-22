import { createSignal } from "solid-js";

/// Whether the buddy list shows the refused-pairing-link notice
/// (`WebviewEvent::PairingLinkRefused`, plan E3.1).
const [pairingLinkRefused, setPairingLinkRefused] = createSignal(false);

export { pairingLinkRefused, setPairingLinkRefused };
