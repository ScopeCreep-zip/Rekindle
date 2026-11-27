import { type Component, Show } from "solid-js";
import { commands } from "../../ipc/commands";
import type { SkinEntry } from "../../ipc/commands/dto";

/**
 * A skin's credits (plan E7.7.3d): its name, original authors,
 * contributors, source and licence, as the Host's `SkinEntry` carries them.
 * An imported skin's credits are what the skin says about itself, so they
 * are labelled as stated by the skin. The source is shown as text and
 * opened only through `open_external_url`, which asks before it opens.
 */
const SkinCredits: Component<{ skin: SkinEntry; id: string }> = (props) => {
  const credits = () => props.skin.credits ?? null;
  const names = (list: string[]): string => (list.length > 0 ? list.join(", ") : "None stated");
  return (
    <section class="skin-credits" id={props.id} aria-label={`Credits for ${props.skin.name}`}>
      <Show when={props.skin.importedAt !== null}>
        <p class="skin-credits-stated">As stated by the skin (not verified)</p>
      </Show>
      <dl class="skin-credits-list">
        <dt class="skin-credits-term">Name</dt>
        <dd class="skin-credits-value">{props.skin.name}</dd>
        <Show
          when={credits()}
          fallback={
            <>
              <dt class="skin-credits-term">Credits</dt>
              <dd class="skin-credits-value">None stated</dd>
            </>
          }
        >
          {(c) => (
            <>
              <dt class="skin-credits-term">Original authors</dt>
              <dd class="skin-credits-value">{names(c().authors)}</dd>
              <dt class="skin-credits-term">Contributors</dt>
              <dd class="skin-credits-value">{names(c().contributors)}</dd>
              <dt class="skin-credits-term">Source</dt>
              <dd class="skin-credits-value">
                <Show when={c().url} fallback="None stated">
                  {(url) => (
                    <>
                      <span class="skin-credits-url">{url()}</span>
                      <button
                        type="button"
                        class="skin-row-btn skin-action"
                        aria-label={`Open the source of ${props.skin.name}`}
                        onClick={() => void commands.openExternalUrl(url())}
                      >
                        Open
                      </button>
                    </>
                  )}
                </Show>
              </dd>
              <dt class="skin-credits-term">Licence</dt>
              <dd class="skin-credits-value">
                {(() => {
                  const licence = c().licence;
                  return licence === "noneStated" ? (
                    "No licence stated"
                  ) : (
                    <section class="skin-credits-licence" tabIndex={0} aria-label="Licence text">
                      {licence.text}
                    </section>
                  );
                })()}
              </dd>
            </>
          )}
        </Show>
      </dl>
    </section>
  );
};

export default SkinCredits;
