import { Component } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import { voiceState } from "../../stores/voice.store";
import { handleToggleDeafen, handleToggleMute } from "../../actions/voice.actions";
import { endUnavailable } from "./voiceEnds";

// Our own mute and deafen in a community call (the voice panel and the call
// bar), read from the Host's `VoiceStatus` (plan E4.1 S5). An end with no
// device keeps its button, visibly disabled with the Host's reason (C7.24:
// listen-only without a microphone, silent without speakers).

export const MuteButton: Component = () => {
  const muted = () => voiceState.status?.muted ?? false;
  const reason = () => endUnavailable(voiceState.status?.input);
  return (
    <button
      class={`voice-btn ${muted() ? "voice-btn-active" : ""}`}
      onClick={handleToggleMute}
      disabled={reason() !== null}
      aria-disabled={reason() !== null}
      title={reason() ?? (muted() ? "Unmute" : "Mute")}
      aria-label={muted() ? "Unmute microphone" : "Mute microphone"}
      aria-pressed={muted()}
    >
      <SkinGlyph name={muted() ? "mic.off" : "mic.on"} />
    </button>
  );
};

export const DeafenButton: Component = () => {
  const deafened = () => voiceState.status?.deafened ?? false;
  const reason = () => endUnavailable(voiceState.status?.output);
  return (
    <button
      class={`voice-btn ${deafened() ? "voice-btn-active" : ""}`}
      onClick={handleToggleDeafen}
      disabled={reason() !== null}
      aria-disabled={reason() !== null}
      title={reason() ?? (deafened() ? "Undeafen" : "Deafen")}
      aria-label={deafened() ? "Undeafen output" : "Deafen output"}
      aria-pressed={deafened()}
    >
      <SkinGlyph name={deafened() ? "speaker.off" : "speaker.on"} />
    </button>
  );
};
