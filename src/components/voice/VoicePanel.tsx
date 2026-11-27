import { type Component, For, Show } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import { voiceState } from "../../stores/voice.store";
import VoiceParticipantItem from "./VoiceParticipant";
import { handleToggleMute, handleToggleDeafen, handleLeaveVoice } from "../../actions/voice.actions";

/// Compact sidebar "connected" strip. Quick mute / deafen / leave plus the
/// participant list — reachable from any channel while in a call. The full
/// control set (camera, screen-share, soundboard, reactions) lives in the
/// main-pane CallStage control bar so this row no longer overflows the rail.
/// Header status string. Community calls surface the three-way join
/// handshake (backend-emitted): announced = our VoiceJoin is out but
/// nobody has seen us; seen = a member acked, confirming; connected =
/// handshake complete (or a non-community call).
function statusLabel(): string {
  if (!voiceState.isConnected) return "Not Connected";
  if (voiceState.activeCallType === "community") {
    if (voiceState.joinHandshake === "announced") return "Voice — waiting for others…";
    if (voiceState.joinHandshake === "seen") return "Voice — connecting…";
  }
  return "Voice Connected";
}

const VoicePanel: Component = () => {
  return (
    <div class="voice-panel">
      <div class="voice-panel-header">
        <span
          class={voiceState.isConnected ? "voice-panel-status" : "voice-panel-status-disconnected"}
        >
          {statusLabel()}
        </span>
        <Show when={voiceState.isConnected}>
          <div class="voice-panel-controls">
            <button
              type="button"
              class={`voice-btn ${voiceState.isMuted ? "voice-btn-active" : ""}`}
              onClick={handleToggleMute}
              title={voiceState.isMuted ? "Unmute" : "Mute"}
              aria-label={voiceState.isMuted ? "Unmute microphone" : "Mute microphone"}
              aria-pressed={voiceState.isMuted}
            >
              <SkinGlyph name={voiceState.isMuted ? "mic.off" : "mic.on"} />
            </button>
            <button
              type="button"
              class={`voice-btn ${voiceState.isDeafened ? "voice-btn-active" : ""}`}
              onClick={handleToggleDeafen}
              title={voiceState.isDeafened ? "Undeafen" : "Deafen"}
              aria-label={voiceState.isDeafened ? "Undeafen output" : "Deafen output"}
              aria-pressed={voiceState.isDeafened}
            >
              <SkinGlyph name={voiceState.isDeafened ? "speaker.off" : "speaker.on"} />
            </button>
            <button
              type="button"
              class="voice-btn voice-btn-disconnect"
              onClick={handleLeaveVoice}
              title="Disconnect"
              aria-label="Leave voice channel"
            >
              <SkinGlyph name="hangup" />
            </button>
          </div>
        </Show>
      </div>

      <Show when={voiceState.isConnected}>
        <div class="voice-participants">
          <For each={voiceState.participants}>
            {(participant) => <VoiceParticipantItem participant={participant} />}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default VoicePanel;
