import { Component, For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import type {
  SkinDraft,
  SkinDraftReport,
  SkinPairCheck,
  TokenValue,
} from "../../ipc/commands/dto";
import { isCommandError } from "../../ipc/commands/dto";

/** `#rrggbb` of an sRGB triple. */
function hex([r, g, b]: [number, number, number]): string {
  return `#${[r, g, b].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

/** The colour picker's `#rrggbb` as an opaque colour token. */
function colour(value: string): TokenValue {
  const n = Number.parseInt(value.slice(1), 16);
  return { type: "color", data: { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255, a: 255 } };
}

const ratio = (x100: number): string => `${(x100 / 100).toFixed(2)}:1`;

function message(e: unknown): string {
  if (isCommandError(e) && e.type === "failed") return e.data.message;
  return e instanceof Error ? e.message : String(e);
}

/** The id of the live-apply stylesheet, in this window's document only. */
const DRAFT_STYLE = "skin-draft-tokens";

/**
 * The skin editor (plan E7.7.5, Telegram's model), a Settings sub-panel:
 * searchable registry keys with a colour picker, live apply to this
 * window only, and beside every declared pair the Host's WCAG ratio and
 * pass/fail (APCA advisory) with its suggested passing colour, applied
 * only by the author's click. Every check is the Host's
 * (`validate_skin_draft`); export is enabled only when the Host's report
 * says every check passes, and the Host's validator gates it again.
 */
const SkinEditor: Component<{ onClose: () => void }> = (props) => {
  const [draft, setDraft] = createSignal<SkinDraft | null>(null);
  const [report, setReport] = createSignal<SkinDraftReport | null>(null);
  const [query, setQuery] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [exported, setExported] = createSignal<string | null>(null);
  let sequence = 0;

  const style = document.createElement("style");
  style.id = DRAFT_STYLE;

  async function check(next: SkinDraft): Promise<void> {
    setDraft(next);
    setExported(null);
    const mine = ++sequence;
    try {
      const r = await commands.validateSkinDraft(next);
      if (mine !== sequence) return;
      setReport(r);
      setError(null);
      style.textContent = r.tokensCss;
      if (!style.isConnected) document.head.append(style);
    } catch (e) {
      if (mine === sequence) setError(message(e));
    }
  }

  async function startFromCurrent(): Promise<void> {
    try {
      await check(await commands.skinDraftStart(null));
    } catch (e) {
      setError(message(e));
    }
  }

  function set(token: string, value: TokenValue): void {
    const d = draft();
    if (d) void check({ ...d, tokens: { ...d.tokens, [token]: value } });
  }

  async function exportDraft(): Promise<void> {
    const d = draft();
    if (!d) return;
    try {
      setExported(await commands.exportSkinDraft(d));
    } catch (e) {
      setError(message(e));
    }
  }

  onMount(() => void startFromCurrent());
  onCleanup(() => style.remove());

  const matches = (text: string): boolean =>
    text.toLowerCase().includes(query().trim().toLowerCase());
  const tokens = createMemo(() =>
    (report()?.tokens ?? []).filter(
      (t) => t.value.type === "color" && (matches(t.id) || matches(t.css)),
    ),
  );
  /** Failing pairs first, then the rest; each filtered by the search. */
  const pairs = createMemo(() => {
    const shown = (report()?.pairs ?? []).filter(
      (p) => matches(p.fg) || matches(p.ground) || matches(p.rule),
    );
    return [...shown.filter((p) => !p.pass), ...shown.filter((p) => p.pass)];
  });
  const failing = (): number => (report()?.pairs ?? []).filter((p) => !p.pass).length;

  const pairRow = (p: SkinPairCheck) => (
    <li class="skin-pair" classList={{ "skin-pair-fail": !p.pass }}>
      <span class="skin-pair-name">
        {p.fg} on {p.ground} <span class="skin-pair-rule">({p.rule})</span>
      </span>
      <span class="skin-pair-ratio">
        {ratio(p.ratioX100)} / {ratio(p.requiredX100)} {p.pass ? "Pass" : "Fail"}
      </span>
      <span class="skin-pair-apca">APCA Lc {(p.apcaLcX10 / 10).toFixed(1)}</span>
      <Show when={p.suggestion}>
        {(s) => (
          <button
            class="form-btn-secondary skin-suggest"
            aria-label={`Use ${hex(s().rgb)} for ${s().token} (${ratio(s().ratioX100)})`}
            onClick={() => set(s().token, colour(hex(s().rgb)))}
          >
            <span class="skin-swatch" style={{ "background-color": hex(s().rgb) }} />
            Use {hex(s().rgb)}
          </button>
        )}
      </Show>
    </li>
  );

  return (
    <div class="skin-editor" aria-label="Skin editor" role="region">
      <div class="skin-editor-bar">
        <input
          class="form-input skin-editor-name"
          aria-label="Skin name"
          value={draft()?.name ?? ""}
          onChange={(e) => {
            const d = draft();
            if (d) void check({ ...d, name: e.currentTarget.value });
          }}
        />
        <button class="form-btn-secondary" onClick={() => void startFromCurrent()}>
          Start from the current skin
        </button>
        <button
          class="form-btn-secondary"
          disabled={!report()?.exportable}
          onClick={() => void exportDraft()}
        >
          Export…
        </button>
        <button class="form-btn-secondary" onClick={() => props.onClose()}>
          Close the editor
        </button>
      </div>
      <div class="settings-hint skin-editor-status" role="status">
        <Show when={report()} fallback="Checking…">
          {(r) =>
            r().exportable
              ? "Every check passes."
              : `${failing() + r().failures.length} checks fail; export is disabled.`
          }
        </Show>
        <Show when={exported()}>{(a) => ` Exported ${a().slice(0, 12)}…`}</Show>
      </div>
      <Show when={error()}>
        {(m) => <div class="settings-hint skin-error" role="alert">{m()}</div>}
      </Show>
      <input
        class="form-input skin-editor-search"
        type="search"
        placeholder="Search keys"
        aria-label="Search keys"
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
      />
      <ul class="skin-editor-failures">
        <For each={report()?.failures ?? []}>{(f) => <li class="settings-hint">{f}</li>}</For>
      </ul>
      <div class="settings-section-title">Keys</div>
      <ul class="skin-tokens">
        <For each={tokens()}>
          {(t) => {
            const v = () => (t.value.type === "color" ? t.value.data : { r: 0, g: 0, b: 0 });
            return (
              <li class="skin-token">
                <input
                  type="color"
                  class="skin-token-picker"
                  aria-label={t.id}
                  value={hex([v().r, v().g, v().b])}
                  onInput={(e) => set(t.id, colour(e.currentTarget.value))}
                />
                <span class="skin-token-id">{t.id}</span>
                <Show when={t.set}>
                  <span class="skin-badge">set</span>
                </Show>
              </li>
            );
          }}
        </For>
      </ul>
      <div class="settings-section-title">Contrast</div>
      <ul class="skin-pairs">
        <For each={pairs()}>{pairRow}</For>
      </ul>
    </div>
  );
};

export default SkinEditor;
