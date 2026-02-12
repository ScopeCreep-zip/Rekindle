import { Component, For, Show } from "solid-js";
import { voiceState } from "../../stores/voice.store";
import VoiceParticipantItem from "./VoiceParticipant";
import { handleToggleMute, handleToggleDeafen, handleLeaveVoice } from "../../handlers/voice.handlers";
import { ICON_MIC, ICON_MIC_OFF, ICON_HEADPHONES, ICON_HEADPHONES_OFF, ICON_HANGUP } from "../../icons";

const VoicePanel: Component = () => {
  return (
    <div class="voice-panel">
      <div class="voice-panel-header">
        <span
          class={voiceState.isConnected ? "voice-panel-status" : "voice-panel-status-disconnected"}
        >
          {voiceState.isConnected ? "Voice Connected" : "Not Connected"}
        </span>
        <Show when={voiceState.isConnected}>
          <div class="voice-panel-controls">
            <button
              class={`voice-btn ${voiceState.isMuted ? "voice-btn-active" : ""}`}
              onClick={handleToggleMute}
              title={voiceState.isMuted ? "Unmute" : "Mute"}
            >
              <span class="nf-icon">
                {voiceState.isMuted ? ICON_MIC_OFF : ICON_MIC}
              </span>
            </button>
            <button
              class={`voice-btn ${voiceState.isDeafened ? "voice-btn-active" : ""}`}
              onClick={handleToggleDeafen}
              title={voiceState.isDeafened ? "Undeafen" : "Deafen"}
            >
              <span class="nf-icon">
                {voiceState.isDeafened ? ICON_HEADPHONES_OFF : ICON_HEADPHONES}
              </span>
            </button>
            <button
              class="voice-btn voice-btn-disconnect"
              onClick={handleLeaveVoice}
              title="Disconnect"
            >
              <span class="nf-icon">{ICON_HANGUP}</span>
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
