import { Component, For, Show } from "solid-js";
import type { SkinDraft, SkinDraftReport } from "../../ipc/commands/dto";

/**
 * The skin's name, description, author and licence, and the credits it
 * carries from the skin it started from (owner 2026-11-18: everyone gets
 * credit for their work). What the export states is the Host's reading
 * (`report.license`, `report.credits`); the Host's validator checks every
 * field, so this only sends the author's edits.
 */
const SkinEditorMeta: Component<{
  draft: SkinDraft;
  report: SkinDraftReport;
  onChange: (draft: SkinDraft) => void;
}> = (props) => {
  const edit = (patch: Partial<SkinDraft>): void => props.onChange({ ...props.draft, ...patch });
  const licenceKind = (): string =>
    "spdx" in props.report.license ? "SPDX expression" : "licence text";
  const original = (): string | null => {
    const l = props.draft.credits.licence;
    return l === "noneStated" ? null : l.text;
  };
  return (
    <div class="skin-editor-meta" role="group" aria-label="Skin details">
      <label class="skin-editor-field">
        <span class="form-label">Name</span>
        <input
          class="form-input"
          value={props.draft.name}
          onChange={(e) => edit({ name: e.currentTarget.value })}
        />
      </label>
      <label class="skin-editor-field">
        <span class="form-label">Author</span>
        <input
          class="form-input"
          placeholder="Your name, as credited"
          value={props.draft.author}
          onChange={(e) => edit({ author: e.currentTarget.value })}
        />
      </label>
      <div class="skin-editor-field">
        <label class="skin-editor-field">
          <span class="form-label">Licence</span>
          <textarea
            class="form-input skin-editor-licence"
            placeholder="An SPDX expression (CC-BY-4.0) or the licence's full text"
            aria-describedby="skin-editor-licence-kind"
            value={props.draft.license}
            onChange={(e) => edit({ license: e.currentTarget.value })}
          />
        </label>
        <span class="settings-hint" id="skin-editor-licence-kind">
          Read as an {licenceKind()}.
        </span>
      </div>
      <label class="skin-editor-field">
        <span class="form-label">Description</span>
        <textarea
          class="form-input"
          value={props.draft.description}
          onChange={(e) => edit({ description: e.currentTarget.value })}
        />
      </label>
      <label class="skin-editor-field">
        <span class="form-label">Web page</span>
        <input
          class="form-input"
          type="url"
          value={props.draft.credits.url ?? ""}
          onChange={(e) =>
            edit({
              credits: { ...props.draft.credits, url: e.currentTarget.value.trim() || null },
            })
          }
        />
      </label>
      <div class="skin-editor-credits" aria-label="Credits the export carries">
        <span class="form-label">Credits</span>
        <ul>
          <For each={props.report.credits.authors}>
            {(name) => <li>{name || "(no author yet)"}</li>}
          </For>
          <For each={props.report.credits.contributors}>
            {(name) => <li>{name} (contributor)</li>}
          </For>
        </ul>
        <Show when={original()}>
          {(text) => (
            <details class="skin-editor-original">
              <summary>Licence of the skin it came from</summary>
              <pre class="skin-credits-licence">{text()}</pre>
            </details>
          )}
        </Show>
      </div>
    </div>
  );
};

export default SkinEditorMeta;
