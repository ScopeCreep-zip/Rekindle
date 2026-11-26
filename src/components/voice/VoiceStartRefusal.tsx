import { Component, Show } from "solid-js";
import { voiceState } from "../../stores/voice.store";

/**
 * Why a new voice session cannot start now, in the Host's words (plan
 * E4.1 S5: one live session per identity across 1:1, group and community
 * calls), or null when one can. Every voice-start button reads it: kept,
 * visibly disabled, the reason as its title. Leave is never refused.
 */
export function voiceStartRefusal(): string | null {
  return voiceState.status?.startRefusal ?? null;
}

/** The start refusal as a status line beside a window's voice-start buttons. */
const VoiceStartRefusal: Component = () => (
  <Show when={voiceStartRefusal()}>
    {(reason) => (
      <div class="voice-start-refusal" role="status">
        {reason()}
      </div>
    )}
  </Show>
);

export default VoiceStartRefusal;
