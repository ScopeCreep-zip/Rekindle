import { Component, For, Show, createMemo, createSignal } from "solid-js";

import { communityState } from "../../stores/community.store";
import {
  handlePlaySoundboard,
  handleUploadEmoji,
  handleUploadSoundboardSound,
  handleUploadSticker,
} from "../../handlers/community.handlers";
import {
  calculateBasePermissions,
  CREATE_EXPRESSIONS,
  hasPermission,
  MANAGE_EXPRESSIONS,
  USE_SOUNDBOARD,
} from "../../ipc/permissions";

type ExpressionTab = "emoji" | "sticker" | "soundboard";

interface ExpressionPickerProps {
  communityId: string;
  /** "reaction" returns "custom:<id>", "message" returns ":name:" or attachment hint. */
  mode: "reaction" | "message";
  searchQuery: string;
  onSelect: (value: string) => void;
  /** Voice channel id, when present, enables soundboard play. */
  activeVoiceChannelId?: string | null;
}

function expressionSelectionValue(mode: "reaction" | "message", expressionId: string, name: string): string {
  if (mode === "reaction") {
    return `custom:${expressionId}`;
  }
  return `:${name}:`;
}

function stickerSelectionValue(mode: "reaction" | "message", expressionId: string, name: string): string {
  if (mode === "reaction") {
    return `custom:${expressionId}`;
  }
  // For messages, stickers attach as image references; the message
  // input layer interprets `sticker:<id>` to insert the asset.
  return `sticker:${expressionId}:${name}`;
}

function sanitizeExpressionName(fileName: string): string {
  const baseName = fileName.replace(/\.[^.]+$/, "");
  const cleaned = baseName.replace(/[^A-Za-z0-9_]+/g, "_").replace(/^_+|_+$/g, "");
  return cleaned.slice(0, 32) || "expression";
}

// Architecture §18.3 — measure duration via Web Audio decodeAudioData.
async function measureAudioDuration(bytes: ArrayBuffer): Promise<number> {
  // Web Audio API's AudioContext is async-only.
  const AudioCtor = (window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext);
  if (!AudioCtor) {
    throw new Error("Web Audio API unavailable in this environment");
  }
  const ctx = new AudioCtor();
  try {
    const decoded = await ctx.decodeAudioData(bytes.slice(0));
    return decoded.duration;
  } finally {
    void ctx.close();
  }
}

const ExpressionPicker: Component<ExpressionPickerProps> = (props) => {
  const [activeTab, setActiveTab] = createSignal<ExpressionTab>("emoji");
  const community = createMemo(() => communityState.communities[props.communityId]);

  const filtered = createMemo(() => {
    const query = props.searchQuery.trim().toLowerCase();
    const all = (community()?.expressions ?? []).filter((expression) => expression.kind === activeTab());
    if (!query) return all;
    return all.filter((expression) => expression.name.toLowerCase().includes(query));
  });

  const myPerms = createMemo(() => {
    const current = community();
    if (!current) return 0n;
    return calculateBasePermissions(current.myRoleIds, current.roles);
  });

  const canUpload = createMemo(() => {
    if (myPerms() === 0n) return false;
    return (
      hasPermission(myPerms(), CREATE_EXPRESSIONS)
      || hasPermission(myPerms(), MANAGE_EXPRESSIONS)
    );
  });

  // Architecture §10.9 — soundboard play requires USE_SOUNDBOARD;
  // backend re-validates on every received SoundboardPlay envelope so
  // a member with the perm revoked mid-session can't keep playing.
  const canPlaySoundboard = createMemo(() =>
    hasPermission(myPerms(), USE_SOUNDBOARD),
  );

  // Soundboard upload is gated more tightly than emoji upload —
  // MANAGE_EXPRESSIONS only, not CREATE_EXPRESSIONS — because sounds
  // are eagerly cached by every member (~48MB budget per community)
  // and a low-trust member could push abusive clips into everyone's
  // cache without this stricter gate.
  const canUploadSoundboard = createMemo(() =>
    hasPermission(myPerms(), MANAGE_EXPRESSIONS),
  );

  async function uploadEmoji(): Promise<void> {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png,image/webp,image/gif";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const animated = file.type === "image/gif";
      await handleUploadEmoji(props.communityId, sanitizeExpressionName(file.name), bytes, animated);
    };
    input.click();
  }

  async function uploadSticker(): Promise<void> {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png,image/webp,image/gif";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const animated = file.type === "image/gif";
      await handleUploadSticker(props.communityId, sanitizeExpressionName(file.name), bytes, animated);
    };
    input.click();
  }

  async function uploadSoundboard(): Promise<void> {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "audio/ogg,audio/mpeg,audio/webm";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const buffer = await file.arrayBuffer();
      let durationSeconds: number;
      try {
        durationSeconds = await measureAudioDuration(buffer);
      } catch (e) {
        console.error("Failed to decode audio:", e);
        return;
      }
      if (durationSeconds <= 0 || durationSeconds > 5) {
        // Backend will also reject; surface client-side first.
        const { addToast } = await import("../../stores/toast.store");
        addToast("Soundboard clips must be 0–5 seconds", "error");
        return;
      }
      const emoji = window.prompt("Optional emoji glyph (leave blank for none)") ?? undefined;
      const bytes = Array.from(new Uint8Array(buffer));
      await handleUploadSoundboardSound(
        props.communityId,
        sanitizeExpressionName(file.name),
        bytes,
        durationSeconds,
        1.0,
        emoji && emoji.trim().length > 0 ? emoji.trim() : undefined,
      );
    };
    input.click();
  }

  function uploadCurrent(): void {
    if (activeTab() === "soundboard") {
      if (!canUploadSoundboard()) return;
      void uploadSoundboard();
      return;
    }
    if (!canUpload()) return;
    if (activeTab() === "emoji") void uploadEmoji();
    else void uploadSticker();
  }

  function selectExpression(expression: { id: string; name: string; kind: string }): void {
    if (expression.kind === "soundboard") {
      // Architecture §10.9 — soundboard plays in the active voice
      // channel rather than inserting into the message body. Frontend
      // gates on USE_SOUNDBOARD; the receiving backend will reject the
      // gossiped SoundboardPlay envelope from any sender lacking the
      // perm so reader-validates kicks in even on a malicious client.
      const ch = props.activeVoiceChannelId;
      if (!ch || !canPlaySoundboard()) return;
      void handlePlaySoundboard(props.communityId, ch, expression.id);
      return;
    }
    if (expression.kind === "sticker") {
      props.onSelect(stickerSelectionValue(props.mode, expression.id, expression.name));
      return;
    }
    props.onSelect(expressionSelectionValue(props.mode, expression.id, expression.name));
  }

  const uploadLabel = createMemo(() =>
    activeTab() === "emoji" ? "Upload emoji" : activeTab() === "sticker" ? "Upload sticker" : "Upload sound",
  );
  const emptyLabel = createMemo(() =>
    activeTab() === "emoji" ? "No custom emoji yet" : activeTab() === "sticker" ? "No stickers yet" : "No soundboard sounds yet",
  );

  return (
    <div class="expression-picker-section">
      <div class="expression-picker-tabs">
        <button
          class={`expression-picker-tab ${activeTab() === "emoji" ? "expression-picker-tab-active" : ""}`}
          onClick={() => setActiveTab("emoji")}
        >
          Emoji
        </button>
        <button
          class={`expression-picker-tab ${activeTab() === "sticker" ? "expression-picker-tab-active" : ""}`}
          onClick={() => setActiveTab("sticker")}
        >
          Stickers
        </button>
        <button
          class={`expression-picker-tab ${activeTab() === "soundboard" ? "expression-picker-tab-active" : ""}`}
          onClick={() => setActiveTab("soundboard")}
        >
          Soundboard
        </button>
      </div>
      <div class="emoji-picker-section-label expression-picker-header">
        <span>Community {activeTab() === "emoji" ? "Emoji" : activeTab() === "sticker" ? "Stickers" : "Sounds"}</span>
        <Show
          when={
            activeTab() === "soundboard"
              ? canUploadSoundboard()
              : canUpload()
          }
        >
          <button class="expression-picker-upload-btn" onClick={uploadCurrent}>
            {uploadLabel()}
          </button>
        </Show>
      </div>
      <Show
        when={filtered().length > 0}
        fallback={<div class="emoji-picker-empty expression-picker-empty">{emptyLabel()}</div>}
      >
        <div class="emoji-picker-grid expression-picker-grid">
          <For each={filtered()}>
            {(expression) => (
              <button
                class="emoji-picker-item expression-picker-item"
                title={
                  expression.kind === "soundboard"
                    ? `${expression.soundMeta?.emoji ?? ""} ${expression.name} (${(expression.soundMeta?.durationSeconds ?? 0).toFixed(1)}s)`
                    : `:${expression.name}:`
                }
                disabled={
                  expression.kind === "soundboard"
                  && (!props.activeVoiceChannelId || !canPlaySoundboard())
                }
                onClick={() => selectExpression(expression)}
              >
                <Show when={expression.kind === "soundboard"} fallback={
                  <Show
                    when={expression.inlineDataUrl}
                    fallback={<span class="expression-picker-fallback">:{expression.name}:</span>}
                  >
                    <img
                      class="expression-picker-image"
                      src={expression.inlineDataUrl!}
                      alt={`:${expression.name}:`}
                    />
                  </Show>
                }>
                  <span class="expression-picker-soundboard-glyph">
                    {expression.soundMeta?.emoji ?? "🔊"}
                  </span>
                  <span class="expression-picker-soundboard-name">{expression.name}</span>
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default ExpressionPicker;
