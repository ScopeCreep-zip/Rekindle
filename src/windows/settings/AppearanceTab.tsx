import { Component, For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import { subscribeSkinEvents, subscribeSkinTrial } from "../../ipc/channels";
import type { SkinEntry, SkinLibrary, SkinTrial } from "../../ipc/commands/dto";
import { isCommandError } from "../../ipc/commands/dto";

/** A Host refusal as the line the tab shows. */
function refusal(e: unknown): string | null {
  if (isCommandError(e)) {
    if (e.type === "failed") return e.data.message;
    // The user cancelled the review: nothing to show.
    if (e.type === "cancelled") return null;
  }
  if (e instanceof Error) return e.message;
  return String(e);
}

/**
 * Settings → Appearance (plan E7.7.2): the skins the Host lists, import,
 * choose, remove and "use as login skin". Every decision is the Host's:
 * choosing a library skin puts it on trial, and this tab only renders the
 * trial it is told about ("Keep this skin?", `SkinTrialChanged`) and sends
 * the user's answer; the Host reverts the skin when the trial runs out.
 */
const AppearanceTab: Component = () => {
  const [library, setLibrary] = createSignal<SkinLibrary | null>(null);
  const [trial, setTrial] = createSignal<SkinTrial | null>(null);
  const [now, setNow] = createSignal(Date.now());
  const [error, setError] = createSignal<string | null>(null);
  let fileInput: HTMLInputElement | undefined;

  const apply = (lib: SkinLibrary): void => {
    setLibrary(lib);
    setTrial(lib.trial);
  };

  async function run(action: () => Promise<SkinLibrary | unknown>): Promise<void> {
    setError(null);
    try {
      const answer = await action();
      if (answer !== null && typeof answer === "object" && "skins" in answer) {
        apply(answer as SkinLibrary);
      } else {
        apply(await commands.listSkins());
      }
    } catch (e) {
      setError(refusal(e));
    }
  }

  const refresh = (): void => void run(() => commands.listSkins());

  onMount(() => {
    refresh();
    const ticker = window.setInterval(() => setNow(Date.now()), 250);
    const trialSub = subscribeSkinTrial((t) => {
      setTrial(t);
      if (t === null) refresh();
    });
    const skinSub = subscribeSkinEvents(() => refresh());
    onCleanup(() => {
      window.clearInterval(ticker);
      void trialSub.then((unlisten) => unlisten());
      void skinSub.then((unlisten) => unlisten());
    });
  });

  /** The skin every window shows now: the trial's, else the stored one. */
  const shown = (): string => trial()?.id ?? library()?.active ?? "";
  const secondsLeft = (t: SkinTrial): number =>
    Math.max(0, Math.ceil((t.revertsAtMs - now()) / 1000));

  async function importFile(file: File): Promise<void> {
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
    await run(() => commands.importSkin(bytes));
  }

  const row = (skin: SkinEntry) => (
    <div class="skin-row" classList={{ "skin-row-shown": shown() === skin.id }}>
      <label class="settings-option skin-choice">
        <input
          type="radio"
          name="skin"
          checked={shown() === skin.id}
          onChange={() => void run(() => commands.selectSkin(skin.id))}
        />
        <span class="buddy-name skin-name">{skin.name}</span>
      </label>
      <Show when={library()?.login === skin.id}>
        <span class="skin-badge">Login skin</span>
      </Show>
      <div class="skin-actions">
        <Show when={library()?.login !== skin.id}>
          <button
            class="form-btn-secondary skin-action"
            onClick={() => void run(() => commands.setLoginSkin(skin.id))}
          >
            Use as login skin
          </button>
        </Show>
        <Show when={skin.importedAt !== null}>
          <button
            class="form-btn-secondary skin-action"
            disabled={shown() === skin.id || library()?.active === skin.id}
            onClick={() => void run(() => commands.removeSkin(skin.id))}
          >
            Remove
          </button>
        </Show>
      </div>
    </div>
  );

  return (
    <>
      <Show when={trial()}>
        {(t) => (
          <section class="sec-review skin-trial" aria-labelledby="skin-trial-title">
            <div class="sec-review-subject" id="skin-trial-title">Keep this skin?</div>
            <p class="skin-trial-text">
              {t().name} goes back to the previous skin in {secondsLeft(t())} s unless you keep it.
            </p>
            <div class="skin-trial-actions">
              <button
                class="sec-btn-commit"
                onClick={() => void run(() => commands.keepSkin(t().id, true))}
              >
                Keep this skin
              </button>
              <button
                class="sec-btn-secondary"
                onClick={() => void run(() => commands.keepSkin(t().id, false))}
              >
                Revert
              </button>
            </div>
          </section>
        )}
      </Show>
      <div class="settings-section-title">Skin</div>
      <div class="skin-list" role="radiogroup" aria-label="Skin">
        <For each={library()?.skins ?? []}>{row}</For>
      </div>
      <div class="skin-import">
        <button class="form-btn-secondary" onClick={() => fileInput?.click()}>
          Import skin…
        </button>
        <input
          ref={fileInput}
          class="skin-file-input"
          type="file"
          accept=".rkskin"
          aria-label="Skin package"
          onChange={(e) => {
            const file = e.currentTarget.files?.[0];
            e.currentTarget.value = "";
            if (file) void importFile(file);
          }}
        />
        <span class="settings-hint">A skin is checked before it is added.</span>
      </div>
      <Show when={error()}>
        {(message) => <div class="settings-hint skin-error" role="alert">{message()}</div>}
      </Show>
    </>
  );
};

export default AppearanceTab;
