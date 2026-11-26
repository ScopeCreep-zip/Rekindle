import { Component, Show } from "solid-js";
import { settingsState } from "../../stores/settings.store";
import { voiceState } from "../../stores/voice.store";
import { endNotice, endUnavailable } from "./voiceEnds";

/** One audio end's line (`voiceEnds.ts`) from the Host's `VoiceStatus`,
 *  shown only when the Host says something about that end. */
export const VoiceEndLine: Component<{ end: "input" | "output" }> = (props) => {
  const state = () => voiceState.status?.[props.end];
  const text = () => endNotice(state(), props.end === "input" ? settingsState.inputDevices : settingsState.outputDevices);
  return (
    <Show when={text()}>
      {(t) => (
        <div class={`voice-end-notice${endUnavailable(state()) !== null ? " voice-end-notice-unavailable" : ""}`} role="status">
          {t()}
        </div>
      )}
    </Show>
  );
};

/** The microphone's line, then the speakers'. */
const VoiceEndNotices: Component = () => (
  <>
    <VoiceEndLine end="input" />
    <VoiceEndLine end="output" />
  </>
);

export default VoiceEndNotices;
