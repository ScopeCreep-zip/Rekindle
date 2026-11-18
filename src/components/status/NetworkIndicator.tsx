import { Component } from "solid-js";
import {
  isNetworkConnected,
  isNetworkConnecting,
  networkLabel,
  networkState,
  trackNetworkStatus,
} from "../../stores/network.store";

const NetworkIndicator: Component = () => {
  trackNetworkStatus();

  function dotClass(): string {
    if (isNetworkConnected()) return "network-dot network-dot-connected";
    if (isNetworkConnecting()) return "network-dot network-dot-connecting";
    if (networkState.publicInternetReady && !networkState.hasRoute) return "network-dot network-dot-partial";
    return "network-dot network-dot-disconnected";
  }

  /**
   * Voice and video need the media route, which the general route never
   * stands in for (plan C7.9c), so while connected without it the
   * indicator says so. `null` once it is available.
   */
  function voiceNote(): string | null {
    if (!isNetworkConnected()) return null;
    switch (networkState.mediaRoute) {
      case "available":
        return null;
      case "failed":
        return "Voice unavailable";
      default:
        return "Voice connecting...";
    }
  }

  function title(): string {
    const note = voiceNote();
    return note ? `Veilid: ${networkLabel()} — ${note}` : `Veilid: ${networkLabel()}`;
  }

  return (
    <div class="network-indicator" title={title()}>
      <div class={dotClass()} />
      <span>{networkLabel()}</span>
      {voiceNote() && <span class="network-voice-note">{voiceNote()}</span>}
    </div>
  );
};

export default NetworkIndicator;
