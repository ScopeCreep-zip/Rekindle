import { Component, createMemo, createSignal, onCleanup, Show } from "solid-js";
import type { MessageAttachment } from "../../stores/chat.store";
import { handleDownloadAttachment } from "../../handlers/community.handlers";
import { ICON_DOWNLOAD } from "../../icons";

interface VoiceMessagePlayerProps {
  communityId: string;
  channelId: string;
  attachment: MessageAttachment;
  /** Voice metadata pulled from the message body's JSON (durationMs +
   *  waveform b64). Architecture §16.4 voice messages carry this in the
   *  carrying Message body so it propagates with the attachment offer. */
  durationMs: number;
  waveform: Uint8Array;
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

const VoiceMessagePlayer: Component<VoiceMessagePlayerProps> = (props) => {
  const [downloading, setDownloading] = createSignal(false);
  const [playing, setPlaying] = createSignal(false);
  const [audio, setAudio] = createSignal<HTMLAudioElement | null>(null);
  const [position, setPosition] = createSignal(0);

  const downloaded = createMemo(() => Boolean(props.attachment.localPath));

  // Render bars from the waveform peak bytes — full vs already-played split
  // is computed from `position / durationMs`.
  const totalBars = createMemo(() => props.waveform.length);
  const playedBars = createMemo(() => {
    if (props.durationMs <= 0) return 0;
    return Math.round((position() / props.durationMs) * totalBars());
  });

  async function ensureDownloaded(): Promise<string | null> {
    if (props.attachment.localPath) return props.attachment.localPath;
    setDownloading(true);
    try {
      const ok = await handleDownloadAttachment(
        props.communityId,
        props.channelId,
        props.attachment.attachmentId,
        props.attachment.filename,
      );
      // After successful download, the AttachmentDownloaded handler updates
      // `attachment.localPath`. If the user cancelled, return null.
      return ok ? props.attachment.localPath ?? null : null;
    } finally {
      setDownloading(false);
    }
  }

  async function togglePlay(): Promise<void> {
    let path = props.attachment.localPath;
    if (!path) {
      path = await ensureDownloaded();
      if (!path) return;
    }
    let el = audio();
    if (!el) {
      const { convertFileSrc } = await import("@tauri-apps/api/core");
      el = new Audio(convertFileSrc(path));
      el.addEventListener("timeupdate", () => setPosition(Math.round(el!.currentTime * 1000)));
      el.addEventListener("ended", () => {
        setPlaying(false);
        setPosition(0);
      });
      setAudio(el);
    }
    if (playing()) {
      el.pause();
      setPlaying(false);
    } else {
      await el.play();
      setPlaying(true);
    }
  }

  onCleanup(() => {
    const el = audio();
    if (el) {
      el.pause();
      el.src = "";
    }
  });

  return (
    <div class="voice-message">
      <button
        class="voice-message-play-btn"
        onClick={() => void togglePlay()}
        disabled={downloading()}
        title={playing() ? "Pause" : downloaded() ? "Play" : "Download + play"}
      >
        <Show
          when={!downloaded() && !downloading()}
          fallback={<span class="nf-icon">{playing() ? "\u{F03E4}" : "\u{F040A}"}</span>}
        >
          <span class="nf-icon">{ICON_DOWNLOAD}</span>
        </Show>
      </button>
      <div class="voice-message-waveform" aria-label="voice message waveform">
        {Array.from({ length: totalBars() }, (_v, i) => {
          const peak = props.waveform[i] ?? 0;
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
        {formatDuration(playing() ? position() : props.durationMs)}
      </span>
    </div>
  );
};

export default VoiceMessagePlayer;
