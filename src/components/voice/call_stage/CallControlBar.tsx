import { Component, Show } from "solid-js";
import { setVoiceState, voiceState } from "../../../stores/voice.store";
import {
  handleToggleMute,
  handleToggleDeafen,
  handleLeaveVoice,
} from "../../../handlers/voice.handlers";
import SoundboardPanel from "../SoundboardPanel";
import ReactionsTray from "../ReactionsTray";
import {
  ICON_MIC,
  ICON_MIC_OFF,
  ICON_HEADPHONES,
  ICON_HEADPHONES_OFF,
  ICON_VIDEO,
  ICON_VIDEO_OFF,
  ICON_SCREEN_SHARE,
  ICON_PIP,
  ICON_HANGUP,
  ICON_CHANNEL_TEXT,
} from "../../../icons";

/// Bottom-centred call controls for the main-pane call stage. Every voice
/// control has room here (the old sidebar strip clipped at ~180px): mute,
/// deafen, camera, screen-share, soundboard, reactions, picture-in-picture,
/// text-chat toggle, and leave — plus a live connection-quality indicator.
const CallControlBar: Component<{
  chatOpen: boolean;
  onToggleChat: () => void;
  onPip?: () => void;
}> = (props) => {
  // Community calls gate video EGRESS on the backend's media-ready
  // state (handshake + roster + MEK + caps + config). The buttons stay
  // enabled — camera/screen start locally (preview) right away and the
  // sender attaches the moment the gate opens; the tooltip says so.
  const previewOnly = (): boolean =>
    voiceState.activeCallType === "community" && !(voiceState.mediaReady?.ready ?? false);
  return (
    <div class="call-control-bar">
      <div
        class="call-control-quality"
        title={`Connection: ${voiceState.connectionQuality} — rx drops 5s: ${voiceState.rxOverflowDrops + voiceState.rxLateDrops}, inbound drops total: ${voiceState.ingressDrops}`}
      >
        <span
          class="call-control-quality-dot"
          classList={{
            "call-control-quality-good": voiceState.connectionQuality === "good",
            "call-control-quality-fair": voiceState.connectionQuality === "fair",
            "call-control-quality-poor": voiceState.connectionQuality === "poor",
          }}
          aria-hidden="true"
        />
        <span class="call-control-quality-label">{voiceState.connectionQuality}</span>
      </div>

      <div class="call-control-group">
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
        <button
          class={`voice-btn ${voiceState.cameraOn ? "voice-btn-active" : ""}`}
          onClick={() => setVoiceState("cameraOn", !voiceState.cameraOn)}
          title={
            voiceState.cameraOn
              ? "Stop camera"
              : previewOnly()
                ? "Start camera (preview only until someone else connects)"
                : "Start camera"
          }
          aria-label={voiceState.cameraOn ? "Stop camera" : "Start camera"}
          aria-pressed={voiceState.cameraOn}
        >
          <span class="nf-icon" aria-hidden="true">
            {voiceState.cameraOn ? ICON_VIDEO : ICON_VIDEO_OFF}
          </span>
        </button>
        <button
          class={`voice-btn ${voiceState.screenShareOn ? "voice-btn-active" : ""}`}
          onClick={() => setVoiceState("screenShareOn", !voiceState.screenShareOn)}
          title={
            voiceState.screenShareOn
              ? "Stop screen share"
              : previewOnly()
                ? "Share screen (preview only until someone else connects)"
                : "Share screen"
          }
          aria-label={voiceState.screenShareOn ? "Stop screen share" : "Share screen"}
          aria-pressed={voiceState.screenShareOn}
        >
          <span class="nf-icon" aria-hidden="true">{ICON_SCREEN_SHARE}</span>
        </button>
      </div>

      <div class="call-control-group">
        <SoundboardPanel />
        <ReactionsTray />
        <Show when={props.onPip}>
          <button
            class="voice-btn"
            onClick={() => props.onPip?.()}
            title="Picture-in-picture"
            aria-label="Pop out video"
          >
            <span class="nf-icon" aria-hidden="true">{ICON_PIP}</span>
          </button>
        </Show>
        <button
          class={`voice-btn ${props.chatOpen ? "voice-btn-active" : ""}`}
          onClick={() => props.onToggleChat()}
          title={props.chatOpen ? "Hide chat" : "Show chat"}
          aria-label={props.chatOpen ? "Hide channel chat" : "Show channel chat"}
          aria-pressed={props.chatOpen}
        >
          <span class="nf-icon" aria-hidden="true">{ICON_CHANNEL_TEXT}</span>
        </button>
      </div>

      <div class="call-control-group">
        <button
          class="voice-btn voice-btn-disconnect"
          onClick={handleLeaveVoice}
          title="Disconnect"
          aria-label="Leave voice channel"
        >
          <span class="nf-icon" aria-hidden="true">{ICON_HANGUP}</span>
        </button>
      </div>
    </div>
  );
};

export default CallControlBar;
