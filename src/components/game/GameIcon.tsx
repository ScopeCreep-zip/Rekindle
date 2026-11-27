import { Component, Show } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";

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
function gameIconUrl(key: string | null | undefined): string | null {
  if (!key) return null;
  return ICONS[`../../assets/game-icons/${key}.png`] ?? null;
}

const GameIcon: Component<{ iconKey: string | null | undefined }> = (props) => (
  <Show
    when={gameIconUrl(props.iconKey)}
    fallback={<SkinGlyph name="gamepad" class="game-icon game-icon-generic" />}
  >
    {(url) => <img class="game-icon" src={url()} alt="" aria-hidden="true" />}
  </Show>
);

export default GameIcon;
