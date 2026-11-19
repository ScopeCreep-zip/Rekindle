import { Component, Show } from "solid-js";
import { ICON_GAMEPAD } from "../../icons";

// Plan E7.2 "Game icons": the backend resolves each game's icon key
// (`xfire/<xfire id>` or `slug/<icon>`) from the game database and puts it
// on the DTO; this renders `src/assets/game-icons/<key>.png`. A key with
// no file (a post-Xfire game until a replacement pack supplies it) shows
// the generic glyph. The icons are emitted as files, never inlined
// (`vite.config.ts` `assetsInlineLimit`).
const ICONS = import.meta.glob<string>("../../assets/game-icons/**/*.png", {
  eager: true,
  query: "?url",
  import: "default",
});

/** The URL of the icon with `key`, or `null` when no file has it. */
export function gameIconUrl(key: string | null | undefined): string | null {
  if (!key) return null;
  return ICONS[`../../assets/game-icons/${key}.png`] ?? null;
}

const GameIcon: Component<{ iconKey: string | null | undefined }> = (props) => (
  <Show
    when={gameIconUrl(props.iconKey)}
    fallback={<span class="game-icon game-icon-generic nf-icon" aria-hidden="true">{ICON_GAMEPAD}</span>}
  >
    {(url) => <img class="game-icon" src={url()} alt="" aria-hidden="true" />}
  </Show>
);

export default GameIcon;
