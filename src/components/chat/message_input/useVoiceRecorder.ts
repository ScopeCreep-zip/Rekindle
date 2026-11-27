import { createSignal, onCleanup } from "solid-js";
import { bytesToBase64 } from "../../../utils/base64";
import { handleSendVoiceMessage } from "../../../actions/community.actions";

/** The longest voice message (`rekindle_files::voice_note::MAX_DURATION_MS`):
 *  the recorder stops at exactly this, and the PCM handed over is cut to it,
 *  so a full-length recording is never refused. */
const VOICE_MESSAGE_MAX_MS = 5 * 60 * 1000;
const VOICE_PCM_MAX_SAMPLES = (VOICE_MESSAGE_MAX_MS / 1000) * 16_000;
/** The rate the Host takes a recording's PCM at
 *  (`rekindle_files::voice_note::INPUT_SAMPLE_RATE`). */
const VOICE_PCM_SAMPLE_RATE = 16_000;

interface VoiceRecorderArgs {
  communityId: () => string | undefined;
  channelId: () => string;
}

// Architecture §16.4 — push-to-record voice messages. Records the mic with
// MediaRecorder in whatever container this webview makes, has the webview
// decode it to mono PCM, and hands the PCM to the Host, which encodes the
// Ogg Opus file and is the only judge of its length and waveform (plan
// E5.1, prod research batch 7 V-1/V-2).
export function useVoiceRecorder(args: VoiceRecorderArgs) {
  let recorder: MediaRecorder | null = null;
  let recorderChunks: BlobPart[] = [];
  let recorderStartedAt = 0;
  let recorderStream: MediaStream | null = null;
  let timerHandle: number | null = null;
  let limitHandle: number | null = null;
  const [recording, setRecording] = createSignal(false);
  const [recordedMs, setRecordedMs] = createSignal(0);

  function teardownRecorder(): void {
    if (timerHandle != null) {
      window.cancelAnimationFrame(timerHandle);
      timerHandle = null;
    }
    if (limitHandle != null) {
      window.clearTimeout(limitHandle);
      limitHandle = null;
    }
    recorderStream?.getTracks().forEach((track) => {
      track.stop();
    });
    recorderStream = null;
    recorder = null;
    recorderChunks = [];
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

      // The cap is a timer of its own, not a frame check, so the stop
      // lands at the limit even while the window is hidden.
      limitHandle = window.setTimeout(() => {
        limitHandle = null;
        void stopRecording(true);
      }, VOICE_MESSAGE_MAX_MS);
      const tick = (): void => {
        setRecordedMs(Math.min(Date.now() - recorderStartedAt, VOICE_MESSAGE_MAX_MS));
        timerHandle = window.requestAnimationFrame(tick);
      };
      timerHandle = window.requestAnimationFrame(tick);
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
    const durationMs = Date.now() - recorderStartedAt;

    const finished: Promise<Blob> = new Promise((resolve) => {
      recorder!.onstop = () => {
        const blob = new Blob(recorderChunks, { type: recorder!.mimeType });
        resolve(blob);
      };
      recorder!.stop();
    });

    setRecording(false);
    const blob = await finished;
    teardownRecorder();
    if (!send || !localCommunityId || durationMs < 200) return; // discard on cancel or sub-200ms
    // The webview decodes its own recording (WebM, MP4 or Ogg) and resamples
    // it to the Host's rate; channel 0 is the mic's mono signal.
    const decoder = new AudioContext({ sampleRate: VOICE_PCM_SAMPLE_RATE });
    try {
      const decoded = await decoder.decodeAudioData(await blob.arrayBuffer());
      // A stop lands a few ms after the limit; the audio is cut to it.
      const pcm = decoded.getChannelData(0).subarray(0, VOICE_PCM_MAX_SAMPLES);
      const pcmB64 = bytesToBase64(new Uint8Array(pcm.buffer, pcm.byteOffset, pcm.byteLength));
      await handleSendVoiceMessage(localCommunityId, localChannelId, pcmB64);
    } catch (e) {
      console.error("voice messages: could not decode the recording:", e);
    } finally {
      await decoder.close().catch(() => {});
    }
  }

  onCleanup(() => teardownRecorder());

  return { recording, recordedMs, startRecording, stopRecording };
}
