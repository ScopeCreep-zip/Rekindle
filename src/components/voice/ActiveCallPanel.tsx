import { type Component, createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import { handleEndDmCall } from "../../actions/calls.actions";
import { commands } from "../../ipc/commands";
import { type CallEntry, callsState } from "../../stores/calls.store";
import { friendsState } from "../../stores/friends.store";
import { setSettingsState, settingsState } from "../../stores/settings.store";
import { setVoiceState, voiceState } from "../../stores/voice.store";
import { initials } from "../../utils/initials";
import SkinGlyph from "../common/SkinGlyph";
import ReactionFloater from "./ReactionFloater";
import ReactionsTray from "./ReactionsTray";
import VideoCallPanel from "./VideoCallPanel";

interface ActiveCallPanelProps {
  call: CallEntry;
  /** "inline" docks under the DM header; "popout" fills the call window. */
  mode?: "inline" | "popout";
}

// Wave 12 W12.5 — full Discord-parity active-call controls. Every action
// hooks an existing backend command:
//   - mute/deafen      → commands.setMute / setDeafen
//   - voice mode       → commands.setVoiceMode (PTT vs voice activity)
//   - device pickers   → commands.setAudioDevices
//   - hangup           → handleEndDmCall
//   - camera toggle    → flips voiceState.cameraOn; VideoCallPanel mounts
//   - speaking ring    → reads from voiceState.participants
//   - timer            → ticks from call.startedAtMs
//   - quality bars     → reads voiceState.connectionQuality (populated by
//                        the backend's ConnectionQuality voice event)
const ActiveCallPanel: Component<ActiveCallPanelProps> = (props) => {
  const [now, setNow] = createSignal(Date.now());
  let interval: ReturnType<typeof setInterval> | undefined;

  createEffect(() => {
    if (props.call) {
      interval = setInterval(() => setNow(Date.now()), 1000);
      onCleanup(() => {
        if (interval) clearInterval(interval);
        interval = undefined;
      });
    }
  });

  const elapsedLabel = (): string => {
    const total = Math.max(0, Math.floor((now() - props.call.startedAtMs) / 1000));
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    const pad = (n: number): string => n.toString().padStart(2, "0");
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
  };

  const isVideoCall = createMemo(() => props.call.kind === "video");

  const peerSpeaking = createMemo(
    () =>
      voiceState.participants.find((p) => p.publicKey === props.call.peerKey)?.isSpeaking ?? false
  );

  const peerStatus = createMemo(() => friendsState.friends[props.call.peerKey]?.status ?? "online");

  const qualityBars = (): number => {
    switch (voiceState.connectionQuality) {
      case "excellent":
        return 4;
      case "good":
        return 3;
      case "fair":
        return 2;
      case "recovering":
        return 2;
      case "poor":
        return 1;
      case "lost":
        return 0;
      default:
        return 3;
    }
  };

  // Wave 12 W12.6 — fire the mid-call media state ping after every
  // toggle so the peer's UI mounts/unmounts tiles in sync. Best-effort:
  // a failed ping is non-fatal because the actual frame stream (or
  // absence of it) is the authoritative truth.
  async function pingMediaState(audio: boolean, video: boolean, screen: boolean): Promise<void> {
    try {
      await commands.sendCallMediaState(props.call.callId, audio, video, screen);
    } catch (e) {
      console.warn("sendCallMediaState failed:", e);
    }
  }

  async function toggleMute(): Promise<void> {
    const next = !voiceState.isMuted;
    setVoiceState("isMuted", next);
    try {
      await commands.setMute(next);
      await pingMediaState(!next, voiceState.cameraOn, voiceState.screenShareOn);
    } catch (e) {
      // Revert UI on failure.
      setVoiceState("isMuted", !next);
      console.error("setMute failed:", e);
    }
  }

  async function toggleDeafen(): Promise<void> {
    const next = !voiceState.isDeafened;
    setVoiceState("isDeafened", next);
    try {
      await commands.setDeafen(next);
    } catch (e) {
      setVoiceState("isDeafened", !next);
      console.error("setDeafen failed:", e);
    }
  }

  function toggleCamera(): void {
    if (!isVideoCall()) return;
    const next = !voiceState.cameraOn;
    setVoiceState("cameraOn", next);
    void pingMediaState(!voiceState.isMuted, next, voiceState.screenShareOn);
  }

  // Wave 12 W12.11 — screen-share toggle. The webview's getDisplayMedia
  // pipeline (VideoCallPanel.startScreen) is driven by the same store
  // flag; we just flip it here and the panel reacts.
  function toggleScreenShare(): void {
    const next = !voiceState.screenShareOn;
    setVoiceState("screenShareOn", next);
    void pingMediaState(!voiceState.isMuted, voiceState.cameraOn, next);
  }

  async function pickInputDevice(deviceName: string): Promise<void> {
    setSettingsState("selectedInputDevice", deviceName || null);
    try {
      await commands.setAudioDevices(deviceName || null, settingsState.selectedOutputDevice);
    } catch (e) {
      console.error("setAudioDevices(input) failed:", e);
    }
  }

  async function pickOutputDevice(deviceName: string): Promise<void> {
    setSettingsState("selectedOutputDevice", deviceName || null);
    try {
      await commands.setAudioDevices(settingsState.selectedInputDevice, deviceName || null);
    } catch (e) {
      console.error("setAudioDevices(output) failed:", e);
    }
  }

  return (
    <div class={`active-call-panel active-call-panel-${props.mode ?? "inline"}`}>
      <ReactionFloater />
      <div class="active-call-panel-header">
        <div
          class={`active-call-peer-avatar${peerSpeaking() ? " active-call-peer-avatar-speaking" : ""}`}
          aria-hidden="true"
        >
          {initials(props.call.displayName)}
        </div>
        <div class="active-call-peer-info">
          <div class="active-call-peer-name">{props.call.displayName}</div>
          <div class="active-call-peer-meta">
            <span class="active-call-kind">{props.call.kind}</span>
            <span class={`active-call-status active-call-status-${peerStatus()}`}>
              {peerStatus()}
            </span>
            <span class="active-call-timer" aria-label="Call duration">
              {elapsedLabel()}
            </span>
          </div>
        </div>
        <div
          class="active-call-quality"
          role="img"
          title={`Connection quality: ${voiceState.connectionQuality}`}
          aria-label={`Connection quality: ${voiceState.connectionQuality}`}
        >
          {[1, 2, 3, 4].map((bar) => (
            <span
              class={`active-call-quality-bar${bar <= qualityBars() ? " active-call-quality-bar-on" : ""}`}
              aria-hidden="true"
            />
          ))}
        </div>
      </div>

      <div class="active-call-controls">
        <button
          type="button"
          class={`call-control-btn${voiceState.isMuted ? " call-control-btn-active" : ""}`}
          onClick={() => void toggleMute()}
          aria-pressed={voiceState.isMuted}
          aria-label={voiceState.isMuted ? "Unmute microphone" : "Mute microphone"}
          title={voiceState.isMuted ? "Unmute" : "Mute"}
        >
          <SkinGlyph name={voiceState.isMuted ? "mic.off" : "mic.on"} />
        </button>
        <button
          type="button"
          class={`call-control-btn${voiceState.isDeafened ? " call-control-btn-active" : ""}`}
          onClick={() => void toggleDeafen()}
          aria-pressed={voiceState.isDeafened}
          aria-label={voiceState.isDeafened ? "Undeafen" : "Deafen"}
          title={voiceState.isDeafened ? "Undeafen" : "Deafen"}
        >
          <SkinGlyph name={voiceState.isDeafened ? "speaker.off" : "speaker.on"} />
        </button>
        <Show when={isVideoCall()}>
          <button
            type="button"
            class={`call-control-btn${voiceState.cameraOn ? " call-control-btn-active" : ""}`}
            onClick={toggleCamera}
            aria-pressed={voiceState.cameraOn}
            aria-label={voiceState.cameraOn ? "Turn off camera" : "Turn on camera"}
            title={voiceState.cameraOn ? "Turn off camera" : "Turn on camera"}
          >
            <SkinGlyph name={voiceState.cameraOn ? "video" : "video-off"} />
          </button>
        </Show>
        {/* Wave 12 W12.11 — screen-share toggle. Available in both audio
         *  and video calls (Discord parity). Capture pipeline lives in
         *  VideoCallPanel; flipping the store flag is what drives it. */}
        <button
          type="button"
          class={`call-control-btn${voiceState.screenShareOn ? " call-control-btn-active" : ""}`}
          onClick={toggleScreenShare}
          aria-pressed={voiceState.screenShareOn}
          aria-label={voiceState.screenShareOn ? "Stop sharing screen" : "Share screen"}
          title={voiceState.screenShareOn ? "Stop sharing screen" : "Share screen"}
        >
          <SkinGlyph name="screen-share" />
        </button>
        {/* Wave 12 W12.11 — emoji reactions tray. */}
        <ReactionsTray />
        {/* Wave 12 W12.7 — pop-out into a dedicated window. Only shown
         *  in the inline mode; the popout itself doesn't show this
         *  button (otherwise it would pop a popout). */}
        <Show when={(props.mode ?? "inline") === "inline"}>
          <button
            type="button"
            class="call-control-btn"
            onClick={() => void commands.openCallWindow(props.call.callId)}
            aria-label="Pop out call"
            title="Pop out"
          >
            <SkinGlyph name="open-in-new" />
          </button>
        </Show>
        <button
          type="button"
          class="call-control-btn call-control-btn-hangup"
          onClick={() => void handleEndDmCall(props.call.callId)}
          aria-label="Hang up"
          title="Hang up"
        >
          <SkinGlyph name="hangup" />
        </button>
      </div>

      {/* Wave 12 W12.6 + W12.11 — mount the panel when EITHER side has
       *  ANY video stream active (camera OR screen share, on either
       *  side) so peer frames have a target as soon as they arrive.
       *  Screen share is available in both audio and video calls per
       *  Discord parity. */}
      <Show
        when={
          voiceState.cameraOn ||
          voiceState.screenShareOn ||
          (props.call.peerMediaState?.video ?? props.call.kind === "video") ||
          (props.call.peerMediaState?.screen ?? false)
        }
      >
        <VideoCallPanel
          mode="dm"
          peerId={props.call.peerKey}
          peerName={props.call.displayName}
          visible
        />
      </Show>

      <details class="active-call-devices">
        <summary class="active-call-devices-summary">
          <SkinGlyph name="phone" />
          Audio devices
        </summary>
        <div class="active-call-device-row">
          <label>Microphone</label>
          <select
            value={settingsState.selectedInputDevice ?? ""}
            onChange={(e) => void pickInputDevice(e.currentTarget.value)}
          >
            <option value="">System default</option>
            {settingsState.inputDevices.map((d) => (
              <option value={d.name}>{d.name}</option>
            ))}
          </select>
        </div>
        <div class="active-call-device-row">
          <label>Speakers</label>
          <select
            value={settingsState.selectedOutputDevice ?? ""}
            onChange={(e) => void pickOutputDevice(e.currentTarget.value)}
          >
            <option value="">System default</option>
            {settingsState.outputDevices.map((d) => (
              <option value={d.name}>{d.name}</option>
            ))}
          </select>
        </div>
      </details>
    </div>
  );
};

export default ActiveCallPanel;

/// Convenience wrapper for inline mounts where the caller has only a
/// peerKey and the active-call store entry needs lookup.
export const ActiveDmCallPanel: Component<{ peerKey: string }> = (props) => {
  const active = createMemo(() => {
    const a = callsState.activeCall;
    return a && a.peerKey === props.peerKey ? a : null;
  });
  return <Show when={active()}>{(call) => <ActiveCallPanel call={call()} mode="inline" />}</Show>;
};
