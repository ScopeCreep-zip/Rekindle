import { Component, For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import { subscribeSkinApplyFailed, subscribeSkinEvents, subscribeSkinTrial, type SkinApplyFailed } from "../../ipc/channels";
import type { SkinEntry, SkinLibrary, SkinTrial } from "../../ipc/commands/dto";
import { isCommandError } from "../../ipc/commands/dto";
import SkinTrialPrompt from "../../components/settings/SkinTrialPrompt";
import SkinCredits from "../../components/settings/SkinCredits";

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

/** Why a window could not show a skin, as one sentence (plan SK2-T1). */
function failureText(f: SkinApplyFailed, name: string): string {
  const why = f.cause.type === "imageDecode"
    ? `its image ${f.cause.data.path.split("/").pop() ?? f.cause.data.path} did not load`
    : "its style sheet did not load";
  const after = f.reverted ? "It was on trial, so the previous skin is back." : "The skin shown before stays.";
  return `${name} could not be shown: ${why}. ${after}`;
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
  /** The newest window's report that a skin could not be applied. */
  const [failed, setFailed] = createSignal<SkinApplyFailed | null>(null);
  /** The skin whose credits are open, if any. */
  const [creditsOf, setCreditsOf] = createSignal<string | null>(null);
  let fileInput: HTMLInputElement | undefined;

  const apply = (lib: SkinLibrary): void => {
    setLibrary(lib);
    setTrial(lib.trial);
  };

  // The newest action's answer stands: a list read issued before a choice
  // (the previous switch's announce) could answer after it and put the old
  // skin back as "in use" while the windows showed the new one (every third
  // switch in a row, settings-theme on both platforms).
  let latest = 0;
  async function run(action: () => Promise<SkinLibrary | unknown>): Promise<void> {
    const mine = ++latest;
    setError(null);
    try {
      const answer = await action();
      const lib = answer !== null && typeof answer === "object" && "skins" in answer
        ? (answer as SkinLibrary)
        : await commands.listSkins();
      if (mine === latest) apply(lib);
    } catch (e) {
      if (mine === latest) setError(refusal(e));
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
    // Every window that applies the skin reports alone; the newest stands.
    const failedSub = subscribeSkinApplyFailed((f) => {
      setFailed(f);
      if (f.reverted) refresh();
    });
    onCleanup(() => {
      window.clearInterval(ticker);
      void trialSub.then((unlisten) => unlisten());
      void skinSub.then((unlisten) => unlisten());
      void failedSub.then((unlisten) => unlisten());
    });
  });

  /** The skin every window shows now: the trial's, else the stored one. */
  const shown = (): string => trial()?.id ?? library()?.active ?? "";
  // Plan SK14: the shown skin's entry, and the conforming built-in the Host
  // lists (the accessible variant), both as served.
  const shownEntry = (): SkinEntry | undefined => library()?.skins.find((s) => s.id === shown());
  const conformingAlternate = (): SkinEntry | undefined =>
    library()?.skins.find((s) => s.conforming && s.importedAt === null && s.id !== shown());
  const secondsLeft = (t: SkinTrial): number =>
    Math.max(0, Math.ceil((t.revertsAtMs - now()) / 1000));

  async function importFile(file: File): Promise<void> {
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
    await run(() => commands.importSkin(bytes));
  }

  const row = (skin: SkinEntry, index: () => number) => (
    <div class="skin-row" classList={{ "skin-row-shown": shown() === skin.id }}>
      <label class="settings-option skin-choice">
        <input
          type="radio"
          name="skin"
          checked={shown() === skin.id}
          aria-describedby={`skin-access-${index()}`}
          onChange={() => void run(() => commands.selectSkin(skin.id))}
        />
        <span class="buddy-name skin-name">{skin.name}</span>
      </label>
      {/* How the skin sizes its controls (plan SK14): a faithful skin keeps
          its art's small controls; a conforming one serves none under 24 px.
          An imported pack is the author's, unchecked by us. */}
      <span class="skin-access" id={`skin-access-${index()}`}>
        {skin.conforming
          ? "Meets target sizes"
          : `${skin.shortTargets} control${skin.shortTargets === 1 ? "" : "s"} under 24 px`}
        {skin.importedAt !== null ? "; an imported skin may not meet accessibility sizes" : ""}
      </span>
      <Show when={library()?.login === skin.id}>
        <span class="skin-badge">Login skin</span>
      </Show>
      <div class="skin-actions">
        <button
          class="skin-row-btn skin-info"
          aria-label={`Credits for ${skin.name}`}
          aria-expanded={creditsOf() === skin.id}
          aria-controls={`skin-credits-${index()}`}
          onClick={() => setCreditsOf(creditsOf() === skin.id ? null : skin.id)}
        >
          i
        </button>
        <Show when={library()?.login !== skin.id}>
          <button
            class="skin-row-btn skin-action"
            onClick={() => void run(() => commands.setLoginSkin(skin.id))}
          >
            Use as login skin
          </button>
        </Show>
        <Show when={skin.importedAt !== null}>
          <button
            class="skin-row-btn skin-action"
            disabled={shown() === skin.id || library()?.active === skin.id}
            onClick={() => void run(() => commands.removeSkin(skin.id))}
          >
            Remove
          </button>
        </Show>
      </div>
      <Show when={creditsOf() === skin.id}>
        <SkinCredits skin={skin} id={`skin-credits-${index()}`} />
      </Show>
    </div>
  );

  return (
    <>
      <Show when={trial()}>
        {(t) => (
          <SkinTrialPrompt
            trial={t()}
            secondsLeft={secondsLeft(t())}
            onAnswer={(keep) => void run(() => commands.keepSkin(t().id, keep))}
          />
        )}
      </Show>
      <Show when={failed()}>
        {(f) => (
          <div class="settings-hint skin-error" role="alert">
            {failureText(f(), library()?.skins.find((s) => s.id === f().skin)?.name ?? f().skin)}{" "}
            <button class="skin-row-btn skin-action" onClick={() => setFailed(null)}>
              Dismiss
            </button>
          </div>
        )}
      </Show>
      <div class="settings-section-title">Skin</div>
      {/* A skin whose controls are smaller than 24 px points to the skin that
          meets the size (plan SK14, WCAG 2.2 5.2.1), one action away. */}
      <Show when={shownEntry() && !shownEntry()!.conforming && conformingAlternate()}>
        {(alt) => (
          <div class="settings-hint skin-alternate">
            {`${shownEntry()!.name} has ${shownEntry()!.shortTargets} control${shownEntry()!.shortTargets === 1 ? "" : "s"} under 24 px. ${alt().name} meets target sizes.`}{" "}
            <button class="skin-row-btn skin-action" onClick={() => void run(() => commands.selectSkin(alt().id))}>
              Use {alt().name}
            </button>
          </div>
        )}
      </Show>
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
        <button
          class="form-btn-secondary"
          onClick={() => void run(() => commands.openSkinEditorWindow())}
        >
          Edit a skin…
        </button>
        <span class="settings-hint">A skin is checked before it is added.</span>
      </div>
      <Show when={error()}>
        {(message) => <div class="settings-hint skin-error" role="alert">{message()}</div>}
      </Show>
    </>
  );
};

export default AppearanceTab;
