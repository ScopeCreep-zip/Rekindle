import { Component, Show } from "solid-js";
import { pairingLinkRefused, setPairingLinkRefused } from "../../stores/pairing-link.store";

/// The in-panel notice for a pairing link the shell refused: a link from a
/// web page or app never pairs a device (plan E3.1, U1). A security surface
/// (plan E7.7.0): the statement sits on the reserved `.sec-review` card.
const PairingLinkNotice: Component = () => (
  <Show when={pairingLinkRefused()}>
    <div class="sec-review" role="status">
      <p class="sec-review-fact">
        A link tried to pair a new device with this account. Pair from Settings with a pairing
        code.
      </p>
      <button class="sec-btn-secondary" onClick={() => setPairingLinkRefused(false)}>
        OK
      </button>
    </div>
  </Show>
);

export default PairingLinkNotice;
