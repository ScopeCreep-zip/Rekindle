import { type Component, For, Show, createSignal } from "solid-js";
import { commands } from "../../ipc/commands";
import type { RegionFit, SkinDraft, SkinDraftReport } from "../../ipc/commands/dto_skin_draft";
import { isCommandError } from "../../ipc/commands/dto";

type FitKind = RegionFit["type"];

/**
 * Region import (plan E7.7.5): a PNG or GIF for a registry region, with
 * its fit and nine-slice insets. The shell asks for the file and the Host
 * runs the draft with it through the same validator as any package, so a
 * bad image or nine-slice is refused by name and the draft is unchanged.
 */
const SkinRegionImport: Component<{
  draft: SkinDraft;
  report: SkinDraftReport;
  onImported: (draft: SkinDraft) => void;
  onError: (message: string) => void;
}> = (props) => {
  const [region, setRegion] = createSignal("");
  const [fit, setFit] = createSignal<FitKind>("nineSlice");
  const [insets, setInsets] = createSignal({ top: 4, right: 4, bottom: 4, left: 4 });
  const [backing, setBacking] = createSignal("");
  const slot = () => props.report.regionSlots.find((s) => s.id === region());
  const colours = () => props.report.tokens.filter((t) => t.value.type === "color");

  const fitValue = (): RegionFit =>
    fit() === "nineSlice"
      ? { type: "nineSlice", data: insets() }
      : { type: fit() as "stretch" | "tile" };

  async function importRegion(): Promise<void> {
    try {
      const next = await commands.importSkinRegion(
        props.draft,
        region(),
        fitValue(),
        slot()?.carriesText ? backing() || null : null
      );
      if (next) props.onImported(next);
    } catch (e) {
      if (isCommandError(e) && e.type === "cancelled") return;
      props.onError(isCommandError(e) && e.type === "failed" ? e.data.message : String(e));
    }
  }

  const inset = (side: "top" | "right" | "bottom" | "left") => (
    <label class="skin-region-inset">
      <span class="form-label">{side}</span>
      <input
        class="form-input"
        type="number"
        min="0"
        value={insets()[side]}
        onInput={(e) => setInsets({ ...insets(), [side]: Number(e.currentTarget.value) || 0 })}
      />
    </label>
  );

  return (
    <section class="skin-region-import" aria-label="Region images">
      <div class="settings-section-title">Regions</div>
      <ul class="skin-regions">
        <For each={Object.entries(props.draft.regions)}>
          {([id, r]) => (
            <li>
              {id}: {r.fit.type}
            </li>
          )}
        </For>
      </ul>
      <div class="skin-region-row">
        <select
          class="form-input"
          aria-label="Region"
          value={region()}
          onChange={(e) => setRegion(e.currentTarget.value)}
        >
          <option value="">Choose a region</option>
          <For each={props.report.regionSlots}>{(s) => <option value={s.id}>{s.id}</option>}</For>
        </select>
        <select
          class="form-input"
          aria-label="Fit"
          value={fit()}
          onChange={(e) => setFit(e.currentTarget.value as FitKind)}
        >
          <option value="nineSlice">Nine-slice</option>
          <option value="stretch">Stretch</option>
          <option value="tile">Tile</option>
        </select>
      </div>
      <Show when={fit() === "nineSlice"}>
        <div class="skin-region-row">
          {inset("top")}
          {inset("right")}
          {inset("bottom")}
          {inset("left")}
        </div>
      </Show>
      <Show when={slot()?.carriesText}>
        <select
          class="form-input"
          aria-label="Text backing"
          value={backing()}
          onChange={(e) => setBacking(e.currentTarget.value)}
        >
          <option value="">Colour under its text</option>
          <For each={colours()}>{(t) => <option value={t.id}>{t.id}</option>}</For>
        </select>
      </Show>
      <button
        type="button"
        class="form-btn-secondary"
        disabled={!region()}
        onClick={() => void importRegion()}
      >
        Import image…
      </button>
    </section>
  );
};

export default SkinRegionImport;
