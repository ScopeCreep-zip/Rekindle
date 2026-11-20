import { onCleanup } from "solid-js";
import { commands } from "../ipc/commands";
import { subscribeSkinEvents } from "../ipc/channels";
import type { SkinApplied } from "../ipc/commands/dto";

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

/// The `<link>` of the newest apply still loading, if any: a newer apply
/// drops it, so a slow sheet can never land over a later choice.
let pending: HTMLLinkElement | null = null;

/**
 * Apply `skin` to this window. The new sheet loads in its own `<link>`
 * beside the shown one: WebKit keeps a link's old `.sheet` until a changed
 * `href` finishes loading, so swapping in place would show the previous
 * skin while the link already names the new one. On `load` the new link
 * takes the id, the old one is removed and `data-skin-base` follows, so the
 * id, the sheet and the base always belong to the same skin. A link a
 * newer apply superseded is dropped; on `error` the shown skin stays and
 * the error is reported.
 */
export function applySkin(skin: SkinApplied): void {
  const shown = document.getElementById(LINK_ID) as HTMLLinkElement | null;
  if (pending !== null) {
    pending.remove();
    pending = null;
  }
  if (shown?.getAttribute("href") === skin.tokensHref) {
    document.documentElement.dataset.skinBase = skin.base;
    return;
  }
  const next = document.createElement("link");
  next.rel = "stylesheet";
  pending = next;
  next.addEventListener("load", () => {
    if (pending !== next) return;
    pending = null;
    document.getElementById(LINK_ID)?.remove();
    next.id = LINK_ID;
    document.documentElement.dataset.skinBase = skin.base;
  });
  next.addEventListener("error", () => {
    if (pending !== next) return;
    pending = null;
    next.remove();
    console.error(`skin sheet ${skin.tokensHref} failed to load — the shown skin stays`);
  });
  next.href = skin.tokensHref;
  document.head.appendChild(next);
}
