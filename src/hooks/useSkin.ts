import { createSignal, onCleanup } from "solid-js";
import { commands } from "../ipc/commands";
import { subscribeSkinEvents } from "../ipc/channels";
import type { SkinApplied } from "../ipc/commands/dto";
import type { SkinShell } from "../ipc/commands/dto_skin_tree";

/// The shown skin's window outlines (plan SK2-S2): set with the sheet that
/// places their nodes, so the nested divs and their rules always belong to
/// the same skin.
const [skinShells, setSkinShells] = createSignal<SkinShell[]>([]);

/** The shown skin's outline for `window`, if it lays that window as a tree. */
export function skinShell(window: SkinShell["window"]): SkinShell | undefined {
  return skinShells().find((s) => s.window === window);
}

/// The one `<link>` each window swaps (plan E7.7.2 "Apply").
const LINK_ID = "rekindle-skin";

/**
 * Plan E7.7.2 "Apply": this window shows the skin the Host resolves (the
 * identity's choice, or before unlock the device login skin). It sets
 * `:root[data-skin-base]` from the active variant and points one
 * `<link rel="stylesheet">` at the skin's `@layer skin` stylesheet on the
 * `skin://` scheme; on every `SkinChanged` it does both again. Nothing
 * goes through `style.setProperty`: element styles would beat every
 * layer, high contrast included. Mounted once per webview, in `main.tsx`.
 */
export function useSkin(): void {
  let stopped = false;
  const unsubscribe = subscribeSkinEvents(applySkin);
  void commands
    .getSkin()
    .then((skin) => {
      if (!stopped) applySkin(skin);
    })
    .catch((err: unknown) => console.error("get_skin failed — this window shows no skin", err));
  onCleanup(() => {
    stopped = true;
    void unsubscribe.then((unlisten) => unlisten());
  });
}

/// Each apply's number: an apply a newer one superseded does nothing more,
/// so a slow sheet or image can never land over a later choice.
let generation = 0;

/// The `<link>` of the newest apply still loading, if any.
let loading: HTMLLinkElement | null = null;

/**
 * Apply `skin` to this window, showing it only once it can be drawn
 * whole. First every image the sheet draws (`skin.images`) is decoded: a
 * frame tile or region still loading paints nothing, so the window would
 * be see-through inside the shaper's cut, which already follows the new
 * skin. Then the sheet loads in its own `<link>` beside the shown one
 * (WebKit keeps a link's old `.sheet` until a changed `href` finishes
 * loading, so swapping in place would show the previous skin while the
 * link already names the new one). On `load` the new link takes the id,
 * the old one is removed and `data-skin-base` follows, so the id, the
 * sheet and the base always belong to the same skin. On any failure the
 * shown skin stays and the error is reported.
 */
export function applySkin(skin: SkinApplied): void {
  const mine = ++generation;
  loading?.remove();
  loading = null;
  const shown = document.getElementById(LINK_ID) as HTMLLinkElement | null;
  if (shown?.getAttribute("href") === skin.tokensHref) {
    document.documentElement.dataset.skinBase = skin.base;
    setSkinShells(skin.shells ?? []);
    return;
  }
  // Held until the swap, so the decoded images stay in this document's cache.
  const images = skin.images.map((href) => {
    const img = new Image();
    img.src = href;
    return img;
  });
  Promise.all(images.map((img) => img.decode())).then(
    () => {
      if (mine === generation) linkSheet(skin, mine, images);
    },
    (err: unknown) => {
      if (mine === generation) console.error(`skin ${skin.id}: an image failed to decode — the shown skin stays`, err);
    },
  );
}

/** Load `skin`'s sheet beside the shown one and swap on `load` (apply `mine`). */
function linkSheet(skin: SkinApplied, mine: number, images: HTMLImageElement[]): void {
  const next = document.createElement("link");
  next.rel = "stylesheet";
  loading = next;
  next.addEventListener("load", () => {
    if (mine !== generation) return;
    loading = null;
    document.getElementById(LINK_ID)?.remove();
    next.id = LINK_ID;
    document.documentElement.dataset.skinBase = skin.base;
    setSkinShells(skin.shells ?? []);
    images.length = 0;
  });
  next.addEventListener("error", () => {
    if (mine !== generation) return;
    loading = null;
    next.remove();
    console.error(`skin sheet ${skin.tokensHref} failed to load — the shown skin stays`);
  });
  next.href = skin.tokensHref;
  document.head.appendChild(next);
}
