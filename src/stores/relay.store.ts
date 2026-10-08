import { createStore } from "solid-js/store";

export interface RelayState {
  /// Friends we've volunteered to relay for, by public key. Drives the
  /// "Stop relaying" vs "Volunteer to relay" label in the context menu.
  volunteeredFor: Record<string, true>;
  /// Friends who have given us a relay route blob (we may use them as
  /// fallbacks when our direct route to a peer fails).
  receivedOffersFrom: Record<string, true>;
}

const [relayState, setRelayState] = createStore<RelayState>({
  volunteeredFor: {},
  receivedOffersFrom: {},
});

export { relayState, setRelayState };
