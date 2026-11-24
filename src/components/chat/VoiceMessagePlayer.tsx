import { Component, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import type { MessageAttachment } from "../../stores/chat.store";
import { handleLoadVoiceMessage } from "../../actions/community.actions";

interface VoiceMessagePlayerProps {
  communityId: string;
  channelId: string;
  attachment: MessageAttachment;
}

const PEAK_BAR_W = 2;
const PEAK_BAR_GAP = 1;
const PEAK_HEIGHT_MIN_PX = 2;
const PEAK_HEIGHT_MAX_PX = 22;

function formatDuration(ms: number): string {
  const total = Math.round(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/// Plays a voice message from its bytes. The audio is fetched through the
/// chunk cache by the backend and played from a `blob:` URL, so nothing is
/// written to disk and no file path reaches the webview.
const VoiceMessagePlayer: Component<VoiceMessagePlayerProps> = (props) => {
  const [fetching, setFetching] = createSignal(false);
  const [playing, setPlaying] = createSignal(false);
  const [audio, setAudio] = createSignal<HTMLAudioElement | null>(null);
  const [position, setPosition] = createSignal(0);
  let objectUrl: string | null = null;
  // Decoded by the Host from the audio itself (plan E5.1), never the
  // sender's claim; unknown until the audio is fetched.
  const [durationMs, setDurationMs] = createSignal(0);
  const [waveform, setWaveform] = createSignal<Uint8Array>(new Uint8Array());

  const loaded = createMemo(() => audio() !== null);

  // Render bars from the waveform peak bytes — full vs already-played split
  // is computed from `position / durationMs`.
  const totalBars = createMemo(() => waveform().length);
  const playedBars = createMemo(() => {
    if (durationMs() <= 0) return 0;
    return Math.round((position() / durationMs()) * totalBars());
  });

  async function loadAudio(): Promise<HTMLAudioElement | null> {
    setFetching(true);
    try {
      const loadedNote = await handleLoadVoiceMessage(
        props.communityId,
        props.channelId,
        props.attachment.attachmentId,
        props.attachment.mimeType,
      );
      if (!loadedNote) return null;
      setDurationMs(loadedNote.durationMs);
      setWaveform(loadedNote.waveform);
      objectUrl = URL.createObjectURL(loadedNote.blob);
      return new Audio(objectUrl);
    } finally {
      setFetching(false);
    }
  }

  async function prepare(): Promise<HTMLAudioElement | null> {
    const existing = audio();
    if (existing || fetching()) return existing;
    const el = await loadAudio();
    if (!el) return null;
    el.addEventListener("timeupdate", () => setPosition(Math.round(el.currentTime * 1000)));
    el.addEventListener("ended", () => {
      setPlaying(false);
      setPosition(0);
    });
    setAudio(el);
    return el;
  }

  async function togglePlay(): Promise<void> {
    const el = await prepare();
    if (!el) return;
    if (playing()) {
      el.pause();
      setPlaying(false);
    } else {
      await el.play();
      setPlaying(true);
    }
  }

  // Fetch on show, as Signal downloads voice notes, so the length and
  // waveform are there before the first play.
  onMount(() => {
    void prepare();
  });

  onCleanup(() => {
    const el = audio();
    if (el) {
      el.pause();
      el.src = "";
    }
    if (objectUrl) URL.revokeObjectURL(objectUrl);
  });

  return (
    <div class="voice-message">
      <button
        class="voice-message-play-btn"
        onClick={() => void togglePlay()}
        disabled={fetching()}
        title={playing() ? "Pause" : loaded() ? "Play" : "Fetch + play"}
      >
        <Show
          when={!loaded() && !fetching()}
          fallback={<SkinGlyph name={playing() ? "pause" : "play"} />}
        >
          <SkinGlyph name="download" />
        </Show>
      </button>
      <div class="voice-message-waveform" aria-label="voice message waveform">
        {Array.from({ length: totalBars() }, (_v, i) => {
          const peak = waveform()[i] ?? 0;
          const heightPx =
            PEAK_HEIGHT_MIN_PX +
            ((PEAK_HEIGHT_MAX_PX - PEAK_HEIGHT_MIN_PX) * peak) / 255;
          const klass = i < playedBars() ? "voice-message-bar voice-message-bar-played" : "voice-message-bar";
          return (
            <span
              class={klass}
              style={{
                width: `${PEAK_BAR_W}px`,
                "margin-right": `${PEAK_BAR_GAP}px`,
                height: `${heightPx}px`,
              }}
            />
          );
        })}
      </div>
      <span class="voice-message-duration">
        {formatDuration(playing() ? position() : durationMs())}
      </span>
    </div>
  );
};

export default VoiceMessagePlayer;
