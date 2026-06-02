import { createSignal, onCleanup } from "solid-js";
import { handleSendVoiceMessage } from "../../../handlers/community.handlers";

const VOICE_MESSAGE_MAX_MS = 5 * 60 * 1000;
const VOICE_WAVEFORM_PEAKS = 64;

interface VoiceRecorderArgs {
  communityId: () => string | undefined;
  channelId: () => string;
}

// Architecture §16.4 — push-to-record voice messages. Captures mic audio
// via MediaRecorder (Opus), samples a live waveform via AnalyserNode, and
// ships the encoded blob + downsampled peaks through
// `handleSendVoiceMessage`.
export function useVoiceRecorder(args: VoiceRecorderArgs) {
  let recorder: MediaRecorder | null = null;
  let recorderChunks: BlobPart[] = [];
  let recorderStartedAt = 0;
  let recorderStream: MediaStream | null = null;
  // Live waveform peaks captured via AnalyserNode while recording.
  let liveAudioCtx: AudioContext | null = null;
  let liveAnalyser: AnalyserNode | null = null;
  let liveSource: MediaStreamAudioSourceNode | null = null;
  let liveSampleHandle: number | null = null;
  let livePeaks: number[] = [];
  const [recording, setRecording] = createSignal(false);
  const [recordedMs, setRecordedMs] = createSignal(0);

  function teardownRecorder(): void {
    if (liveSampleHandle != null) {
      window.cancelAnimationFrame(liveSampleHandle);
      liveSampleHandle = null;
    }
    liveAnalyser = null;
    liveSource?.disconnect();
    liveSource = null;
    liveAudioCtx?.close().catch(() => {});
    liveAudioCtx = null;
    recorderStream?.getTracks().forEach((t) => t.stop());
    recorderStream = null;
    recorder = null;
    recorderChunks = [];
  }

  function downsamplePeaksTo(target: number, peaks: number[]): Uint8Array {
    if (peaks.length === 0) return new Uint8Array();
    const out = new Uint8Array(target);
    if (peaks.length <= target) {
      for (let i = 0; i < peaks.length; i++) out[i] = peaks[i];
      return out.slice(0, peaks.length);
    }
    const stride = peaks.length / target;
    for (let i = 0; i < target; i++) {
      const start = Math.floor(i * stride);
      const end = Math.min(peaks.length, Math.floor((i + 1) * stride));
      let max = 0;
      for (let j = start; j < end; j++) max = Math.max(max, peaks[j]);
      out[i] = max;
    }
    return out;
  }

  function bytesToBase64(bytes: Uint8Array): string {
    let binary = "";
    for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    return btoa(binary);
  }

  async function startRecording(): Promise<void> {
    if (!args.communityId() || recording()) return;
    if (!navigator.mediaDevices?.getUserMedia) {
      console.warn("voice messages: getUserMedia unavailable");
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      recorderStream = stream;
      const mimeCandidates = [
        "audio/ogg; codecs=opus",
        "audio/webm; codecs=opus",
        "audio/webm",
      ];
      const mime = mimeCandidates.find((m) => MediaRecorder.isTypeSupported(m)) ?? "";
      recorder = new MediaRecorder(stream, mime ? { mimeType: mime } : undefined);
      recorderChunks = [];
      recorder.ondataavailable = (e) => {
        if (e.data && e.data.size > 0) recorderChunks.push(e.data);
      };
      recorder.start();
      recorderStartedAt = Date.now();
      setRecording(true);
      setRecordedMs(0);

      // Hook AnalyserNode for live peak sampling.
      liveAudioCtx = new AudioContext();
      liveAnalyser = liveAudioCtx.createAnalyser();
      liveAnalyser.fftSize = 256;
      liveSource = liveAudioCtx.createMediaStreamSource(stream);
      liveSource.connect(liveAnalyser);
      const buf = new Uint8Array(liveAnalyser.frequencyBinCount);
      livePeaks = [];
      const tick = (): void => {
        if (!liveAnalyser) return;
        liveAnalyser.getByteTimeDomainData(buf);
        let peak = 0;
        for (let i = 0; i < buf.length; i++) {
          const v = Math.abs(buf[i] - 128);
          if (v > peak) peak = v;
        }
        livePeaks.push(Math.min(255, peak * 2));
        setRecordedMs(Date.now() - recorderStartedAt);
        if (Date.now() - recorderStartedAt >= VOICE_MESSAGE_MAX_MS) {
          void stopRecording(true);
          return;
        }
        liveSampleHandle = window.requestAnimationFrame(tick);
      };
      liveSampleHandle = window.requestAnimationFrame(tick);
    } catch (e) {
      console.error("voice messages: failed to start recording:", e);
      teardownRecorder();
      setRecording(false);
    }
  }

  async function stopRecording(send: boolean): Promise<void> {
    if (!recorder || !recording()) {
      teardownRecorder();
      setRecording(false);
      return;
    }
    const localCommunityId = args.communityId();
    const localChannelId = args.channelId();
    const peaksAtStop = downsamplePeaksTo(VOICE_WAVEFORM_PEAKS, livePeaks);
    const durationMs = Date.now() - recorderStartedAt;

    const finished: Promise<Blob> = new Promise((resolve) => {
      recorder!.onstop = () => {
        const blob = new Blob(recorderChunks, { type: recorder!.mimeType || "audio/ogg" });
        resolve(blob);
      };
      recorder!.stop();
    });

    setRecording(false);
    const blob = await finished;
    teardownRecorder();
    if (!send || !localCommunityId || durationMs < 200) return; // discard on cancel or sub-200ms
    const buf = new Uint8Array(await blob.arrayBuffer());
    const opusB64 = bytesToBase64(buf);
    const waveformB64 = bytesToBase64(peaksAtStop);
    await handleSendVoiceMessage(localCommunityId, localChannelId, opusB64, durationMs, waveformB64);
  }

  onCleanup(() => teardownRecorder());

  return { recording, recordedMs, startRecording, stopRecording };
}
