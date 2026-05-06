import { Component, For, createSignal, createEffect, createMemo, onCleanup, Show } from "solid-js";
import { Popover } from "@kobalte/core/popover";
import { handleKeyDown } from "../../handlers/chat.handlers";
import { handleUploadAttachment, handleSendVoiceMessage } from "../../handlers/community.handlers";
import EmojiPicker from "./EmojiPicker";
import ReplyPreview from "./ReplyPreview";
import { communityState } from "../../stores/community.store";
import { voiceState } from "../../stores/voice.store";
import { calculateBasePermissions, hasPermission, MENTION_EVERYONE } from "../../ipc/permissions";
import { ICON_CLOSE, ICON_EMOTICON, ICON_PAPERCLIP } from "../../icons";

// Architecture §28.5 — mentions: `@user`, `@role`, `@everyone`,
// `@here`. Backend resolves by display-name (members) or role-name
// (mentionable roles), with permission gating in
// `services/community/mentions.rs::validate_sender_permissions`.
interface MentionCandidate {
  /** Token to splice into the message body, e.g. "alice" or "everyone". */
  token: string;
  /** Display label in the picker. */
  label: string;
  /** "member" / "role" / "special" — categorizes the picker rows. */
  kind: "member" | "role" | "special";
}

const ICON_MIC = "\u{F036C}"; // nf-md-microphone
const ICON_MIC_OFF = "\u{F036D}"; // nf-md-microphone_off
const VOICE_MESSAGE_MAX_MS = 5 * 60 * 1000;
const VOICE_WAVEFORM_PEAKS = 64;

export interface EditMode {
  messageId: string;
  body: string;
}

interface MessageInputProps {
  communityId?: string;
  peerId: string;
  replyTo?: { senderName: string; body: string; messageId?: string } | null;
  editMode?: EditMode | null;
  onSend?: (id: string, body: string, replyToId?: string) => void;
  onDismissReply?: () => void;
  onEditSave?: (messageId: string, newBody: string) => void;
  onEditCancel?: () => void;
  onTyping?: () => void;
  disabled?: boolean;
  disabledMessage?: string;
  slowmodeSeconds?: number;
  bypassSlowmode?: boolean;
}

// Persists the most recent successful-send timestamp per channel/peer across
// MessageInput remounts (switching channels destroys the component but the map
// is module-scoped so the cooldown survives).
const lastSendAtMs = new Map<string, number>();

const MessageInput: Component<MessageInputProps> = (props) => {
  const [body, setBody] = createSignal("");
  const [showEmojiPicker, setShowEmojiPicker] = createSignal(false);
  const [now, setNow] = createSignal(Date.now());
  // Mention autocomplete state — `mentionQuery == null` means no
  // active picker (last keystroke wasn't preceded by a `@` token).
  const [mentionQuery, setMentionQuery] = createSignal<string | null>(null);
  const [mentionSelected, setMentionSelected] = createSignal(0);
  let textareaRef: HTMLTextAreaElement | undefined;

  // Build the candidate list once per query change. Limited to 8 rows
  // so the popover never overruns the input.
  const mentionCandidates = createMemo<MentionCandidate[]>(() => {
    const q = mentionQuery();
    if (q == null || !props.communityId) return [];
    const community = communityState.communities[props.communityId];
    if (!community) return [];
    const lc = q.toLowerCase();
    const matches = (text: string): boolean =>
      lc.length === 0 || text.toLowerCase().startsWith(lc);

    const candidates: MentionCandidate[] = [];

    // Architecture §17.2 + §28.5 — @everyone / @here gate locally;
    // sender's MENTION_EVERYONE bit determines whether the receiver
    // actually escalates the notification, but we surface them in the
    // picker for everyone (the literal text always renders).
    const perms = calculateBasePermissions(community.myRoleIds, community.roles);
    const canMentionEveryone = hasPermission(perms, MENTION_EVERYONE);
    if (canMentionEveryone) {
      if (matches("everyone")) {
        candidates.push({ token: "everyone", label: "@everyone — every member", kind: "special" });
      }
      if (matches("here")) {
        candidates.push({ token: "here", label: "@here — every online member", kind: "special" });
      }
    }

    // @role mentions — mentionable flag gates discovery.
    for (const role of community.roles ?? []) {
      if (!role.mentionable) continue;
      if (matches(role.name)) {
        candidates.push({
          token: role.name,
          label: `@${role.name} (role)`,
          kind: "role",
        });
        if (candidates.length >= 8) break;
      }
    }

    // @member mentions — display name lookup.
    if (candidates.length < 8) {
      for (const member of community.members ?? []) {
        if (matches(member.displayName)) {
          candidates.push({
            token: member.displayName,
            label: `@${member.displayName}`,
            kind: "member",
          });
          if (candidates.length >= 8) break;
        }
      }
    }

    return candidates;
  });

  function findMentionContext(value: string, caretIndex: number): string | null {
    // Look back from caret for the start of the active mention token.
    // A mention starts at `@` preceded by start-of-input, whitespace,
    // or punctuation — anything else means the user typed an email or
    // similar.
    let i = caretIndex - 1;
    while (i >= 0) {
      const ch = value[i];
      if (ch === "@") {
        const prev = i > 0 ? value[i - 1] : "";
        if (i === 0 || /[\s.,;:!?(){}\[\]]/.test(prev)) {
          // Reject if there's a space between @ and caret — that means the
          // user has moved past a completed mention.
          const slice = value.slice(i + 1, caretIndex);
          if (/[\s]/.test(slice)) return null;
          return slice;
        }
        return null;
      }
      if (/[\s\n]/.test(ch)) return null;
      i -= 1;
    }
    return null;
  }

  function applyMention(candidate: MentionCandidate): void {
    const ta = textareaRef;
    if (!ta) return;
    const value = body();
    const caret = ta.selectionStart ?? value.length;
    // Walk back to the `@` that opened the picker.
    let start = caret - 1;
    while (start >= 0 && value[start] !== "@") start -= 1;
    if (start < 0) return;
    const before = value.slice(0, start);
    const after = value.slice(caret);
    const replacement = `@${candidate.token} `;
    const next = `${before}${replacement}${after}`;
    setBody(next);
    setMentionQuery(null);
    // Restore caret position right after the inserted token + space.
    queueMicrotask(() => {
      const newCaret = start + replacement.length;
      ta.setSelectionRange(newCaret, newCaret);
      ta.focus();
    });
  }

  // When entering edit mode, populate the input with the message body
  createEffect(() => {
    const edit = props.editMode;
    if (edit) {
      setBody(edit.body);
    }
  });

  // Tick once per second so the countdown updates. Only when slowmode applies.
  createEffect(() => {
    const seconds = props.slowmodeSeconds ?? 0;
    if (seconds <= 0 || props.bypassSlowmode) return;
    const interval = window.setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => window.clearInterval(interval));
  });

  const cooldownRemainingMs = createMemo(() => {
    const seconds = props.slowmodeSeconds ?? 0;
    if (seconds <= 0 || props.bypassSlowmode) return 0;
    const last = lastSendAtMs.get(props.peerId) ?? 0;
    const cooldown = seconds * 1000;
    const elapsed = now() - last;
    return Math.max(0, cooldown - elapsed);
  });

  const cooldownActive = createMemo(() => cooldownRemainingMs() > 0);
  const cooldownLabel = createMemo(() => {
    const remaining = cooldownRemainingMs();
    if (remaining <= 0) return "";
    const seconds = Math.ceil(remaining / 1000);
    return `Slowmode: wait ${seconds}s before sending`;
  });

  const effectiveDisabled = createMemo(() =>
    Boolean(props.disabled) || (cooldownActive() && !props.editMode),
  );
  const effectiveDisabledMessage = createMemo(() => {
    if (cooldownActive() && !props.editMode) return cooldownLabel();
    return props.disabledMessage ?? "You cannot send messages here";
  });

  function getBody(): string {
    return body();
  }

  function clearInput(): void {
    setBody("");
  }

  function recordSent(): void {
    if ((props.slowmodeSeconds ?? 0) > 0 && !props.bypassSlowmode) {
      lastSendAtMs.set(props.peerId, Date.now());
      setNow(Date.now());
    }
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (effectiveDisabled()) return;

    // Mention picker absorbs ArrowUp/Down/Enter/Tab/Escape when open.
    if (mentionQuery() !== null) {
      const candidates = mentionCandidates();
      if (e.key === "Escape") {
        e.preventDefault();
        setMentionQuery(null);
        return;
      }
      if (candidates.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setMentionSelected((i) => (i + 1) % candidates.length);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setMentionSelected((i) => (i - 1 + candidates.length) % candidates.length);
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          applyMention(candidates[mentionSelected()]);
          return;
        }
      }
    }

    // Edit mode: Escape cancels, Enter saves
    if (props.editMode) {
      if (e.key === "Escape") {
        e.preventDefault();
        props.onEditCancel?.();
        clearInput();
        return;
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const text = getBody().trim();
        if (text && text !== props.editMode.body) {
          props.onEditSave?.(props.editMode.messageId, text);
        } else {
          props.onEditCancel?.();
        }
        clearInput();
        return;
      }
      return;
    }

    if (props.onSend) {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const text = getBody();
        if (text.trim()) {
          props.onSend(props.peerId, text, props.replyTo?.messageId);
          clearInput();
          props.onDismissReply?.();
          recordSent();
        }
      }
    } else {
      handleKeyDown(e, props.peerId, getBody, clearInput);
    }
  }

  function onInput(e: InputEvent): void {
    const target = e.target as HTMLTextAreaElement;
    setBody(target.value);
    // Recompute mention picker query from the new caret position.
    const ctx = findMentionContext(target.value, target.selectionStart ?? target.value.length);
    setMentionQuery(ctx);
    setMentionSelected(0);
    props.onTyping?.();
  }

  function insertEmoji(value: string): void {
    setBody((current) => `${current}${value}`);
    setShowEmojiPicker(false);
  }

  async function handleAttachClick(): Promise<void> {
    if (!props.communityId) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ multiple: false, directory: false });
    if (!picked) return;
    await handleUploadAttachment(props.communityId, props.peerId, picked as string);
  }

  // ─── Voice messages (architecture §16.4) ───────────────────────────
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
    if (!props.communityId || recording()) return;
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
    const localCommunityId = props.communityId;
    const localChannelId = props.peerId;
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
    await handleSendVoiceMessage(
      localCommunityId,
      localChannelId,
      opusB64,
      durationMs,
      waveformB64,
    );
  }

  onCleanup(() => teardownRecorder());

  return (
    <div class="message-input-wrapper">
      <Show when={props.editMode}>
        <div class="edit-mode-header">
          <span>Editing message</span>
          <button
            class="edit-mode-cancel"
            onClick={() => { props.onEditCancel?.(); clearInput(); }}
            title="Cancel edit (Esc)"
            aria-label="Cancel edit"
          >
            <span class="nf-icon" aria-hidden="true">{ICON_CLOSE}</span>
          </button>
        </div>
      </Show>
      <Show when={!props.editMode}>
        <ReplyPreview
          replyTo={props.replyTo ?? null}
          onDismiss={() => props.onDismissReply?.()}
        />
      </Show>
      <Show when={effectiveDisabled() && !props.editMode}>
        <div class="typing-indicator">
          <span class="typing-label">{effectiveDisabledMessage()}</span>
        </div>
      </Show>
      <Show when={!effectiveDisabled() || props.editMode}>
        <div class="message-input-shell">
          <Show when={props.communityId && !props.editMode}>
            <button
              class="message-input-attach-btn"
              type="button"
              title="Attach file"
              aria-label="Attach file"
              onClick={() => void handleAttachClick()}
            >
              <span class="nf-icon" aria-hidden="true">{ICON_PAPERCLIP}</span>
            </button>
          </Show>
          <Show when={props.communityId && !props.editMode}>
            <button
              class={`message-input-mic-btn ${recording() ? "message-input-mic-btn-active" : ""}`}
              type="button"
              title={recording() ? "Release to send (mousedown to record)" : "Hold to record voice"}
              aria-label={recording() ? "Recording voice — release to send" : "Hold to record a voice message"}
              aria-pressed={recording()}
              onMouseDown={() => void startRecording()}
              onMouseUp={() => void stopRecording(true)}
              onMouseLeave={() => recording() && void stopRecording(true)}
              onTouchStart={() => void startRecording()}
              onTouchEnd={() => void stopRecording(true)}
            >
              <span class="nf-icon" aria-hidden="true">{recording() ? ICON_MIC_OFF : ICON_MIC}</span>
              <Show when={recording()}>
                <span class="message-input-mic-timer">
                  {Math.floor(recordedMs() / 1000)}s
                </span>
              </Show>
            </button>
          </Show>
          {/* Plan §Failure 3 — Kobalte Popover with Popover.Portal so the
           * picker isn't clipped by `.message-input-area`'s flex/overflow
           * box. Same pattern as MessageBubble's react picker. */}
          <Popover open={showEmojiPicker()} onOpenChange={setShowEmojiPicker}>
            <Popover.Trigger
              as="button"
              class="message-input-emoji-btn"
              type="button"
              title="Insert emoji"
              aria-label="Insert emoji"
              aria-pressed={showEmojiPicker()}
            >
              <span class="nf-icon" aria-hidden="true">{ICON_EMOTICON}</span>
            </Popover.Trigger>
            <Popover.Portal>
              <Popover.Content class="emoji-picker-popover">
                <EmojiPicker
                  communityId={props.communityId}
                  mode="message"
                  activeVoiceChannelId={voiceState.activeCallType === "community" ? voiceState.channelId : null}
                  onSelect={insertEmoji}
                  onClose={() => setShowEmojiPicker(false)}
                />
              </Popover.Content>
            </Popover.Portal>
          </Popover>
          <textarea
            ref={(el) => (textareaRef = el)}
            class={`message-input message-input-field ${props.editMode ? "message-input-editing" : ""}`}
            placeholder={props.editMode ? "Edit your message..." : "Type a message..."}
            value={body()}
            onInput={onInput}
            onKeyDown={onKeyDown}
            rows={2}
            role="combobox"
            aria-autocomplete="list"
            aria-expanded={mentionQuery() !== null && mentionCandidates().length > 0}
            aria-controls="mention-popover-list"
            aria-activedescendant={
              mentionQuery() !== null && mentionCandidates().length > 0
                ? `mention-row-${mentionSelected()}`
                : undefined
            }
          />
          {/* Architecture §28.5 — mention autocomplete popover.
           * Implements the ARIA combobox listbox pattern (2026-canon):
           * arrow keys move `mentionSelected`, Enter applies, Esc
           * dismisses; the textarea uses `aria-activedescendant` to
           * point at the focused row without moving DOM focus. */}
          {/* Architecture §28.5 — ARIA combobox/listbox: each row is a
           * `<li role="option">` carrying the id `aria-activedescendant`
           * resolves to. No nested button — the textarea above owns focus
           * and arrow keys drive selection; mouse interactions live on
           * the option element itself so the focus target and the id-
           * referenced node are the same element. */}
          <Show when={mentionQuery() !== null && mentionCandidates().length > 0}>
            <ul class="mention-popover" role="listbox" id="mention-popover-list">
              <For each={mentionCandidates()}>
                {(candidate, index) => (
                  <li
                    role="option"
                    id={`mention-row-${index()}`}
                    aria-selected={index() === mentionSelected()}
                    class={`mention-popover-row ${index() === mentionSelected() ? "mention-popover-row-active" : ""}`}
                    onMouseEnter={() => setMentionSelected(index())}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      applyMention(candidate);
                    }}
                  >
                    <span class={`mention-popover-tag mention-popover-tag-${candidate.kind}`}>{candidate.kind}</span>
                    <span class="mention-popover-label">{candidate.label}</span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default MessageInput;
