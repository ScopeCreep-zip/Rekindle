import { Component, For, Show } from "solid-js";
import { setVoiceState, voiceState } from "../../stores/voice.store";
import VoiceParticipantItem from "./VoiceParticipant";
import SoundboardPanel from "./SoundboardPanel";
import { handleToggleMute, handleToggleDeafen, handleLeaveVoice } from "../../handlers/voice.handlers";
import {
  ICON_MIC,
  ICON_MIC_OFF,
  ICON_HEADPHONES,
  ICON_HEADPHONES_OFF,
  ICON_HANGUP,
  ICON_VIDEO,
  ICON_VIDEO_OFF,
  ICON_SCREEN_SHARE,
} from "../../icons";

function toggleCamera(): void {
  setVoiceState("cameraOn", !voiceState.cameraOn);
}

function toggleScreenShare(): void {
  setVoiceState("screenShareOn", !voiceState.screenShareOn);
}

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
              aria-label={voiceState.isMuted ? "Unmute microphone" : "Mute microphone"}
              aria-pressed={voiceState.isMuted}
            >
              <span class="nf-icon" aria-hidden="true">
                {voiceState.isMuted ? ICON_MIC_OFF : ICON_MIC}
              </span>
            </button>
            <button
              class={`voice-btn ${voiceState.isDeafened ? "voice-btn-active" : ""}`}
              onClick={handleToggleDeafen}
              title={voiceState.isDeafened ? "Undeafen" : "Deafen"}
              aria-label={voiceState.isDeafened ? "Undeafen output" : "Deafen output"}
              aria-pressed={voiceState.isDeafened}
            >
              <span class="nf-icon" aria-hidden="true">
                {voiceState.isDeafened ? ICON_HEADPHONES_OFF : ICON_HEADPHONES}
              </span>
            </button>
            {/* Architecture §10.6 — camera + screen-share toggles. The
             * desired-state lives in `voice.store.ts::cameraOn` /
             * `screenShareOn`; VideoCallPanel reacts via createEffect
             * to start/stop the WebCodecs pipeline. Toggle buttons sit
             * in VoicePanel so they're always visible alongside the
             * other call controls. */}
            <button
              class={`voice-btn ${voiceState.cameraOn ? "voice-btn-active" : ""}`}
              onClick={toggleCamera}
              title={voiceState.cameraOn ? "Stop camera" : "Start camera"}
              aria-label={voiceState.cameraOn ? "Stop camera" : "Start camera"}
              aria-pressed={voiceState.cameraOn}
            >
              <span class="nf-icon" aria-hidden="true">
                {voiceState.cameraOn ? ICON_VIDEO : ICON_VIDEO_OFF}
              </span>
            </button>
            <button
              class={`voice-btn ${voiceState.screenShareOn ? "voice-btn-active" : ""}`}
              onClick={toggleScreenShare}
              title={voiceState.screenShareOn ? "Stop screen share" : "Share screen"}
              aria-label={voiceState.screenShareOn ? "Stop screen share" : "Share screen"}
              aria-pressed={voiceState.screenShareOn}
            >
              <span class="nf-icon" aria-hidden="true">{ICON_SCREEN_SHARE}</span>
            </button>
            <button
              class="voice-btn voice-btn-disconnect"
              onClick={handleLeaveVoice}
              title="Disconnect"
              aria-label="Leave voice channel"
            >
              <span class="nf-icon" aria-hidden="true">{ICON_HANGUP}</span>
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
        {/* Plan §Failure 6 — soundboard alongside mute/deafen. Internally
         *  gates on USE_SOUNDBOARD and on the active community having at
         *  least one soundboard Expression, so non-permission users
         *  see nothing rather than an empty panel. */}
        <SoundboardPanel />
      </Show>
    </div>
  );
};

export default VoicePanel;
