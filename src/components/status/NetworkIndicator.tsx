import { Component, For } from "solid-js";
import {
  isNetworkConnected,
  isNetworkConnecting,
  networkLabel,
  networkState,
  trackNetworkStatus,
} from "../../stores/network.store";

/** Veilid's five attached levels ("signal strength"), one bar each. */
const BAR_COUNT = 5;
const BAR_LEVELS = [1, 2, 3, 4, 5];

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

  /** Veilid's attachment strength, as the backend smoothed it. */
  function barsLabel(): string {
    return `Signal ${networkState.bars} of ${BAR_COUNT}`;
  }

  function title(): string {
    const note = voiceNote();
    const head = `Veilid: ${networkLabel()}, ${barsLabel()}`;
    return note ? `${head} — ${note}` : head;
  }

  return (
    <div class="network-indicator" title={title()}>
      <div class={dotClass()} />
      <div class="network-bars" role="img" aria-label={barsLabel()}>
        <For each={BAR_LEVELS}>
          {(level) => (
            <div
              class="network-bar"
              classList={{ "network-bar-on": networkState.bars >= level }}
              data-level={level}
            />
          )}
        </For>
      </div>
      <span>{networkLabel()}</span>
      {voiceNote() && <span class="network-voice-note">{voiceNote()}</span>}
    </div>
  );
};

export default NetworkIndicator;
