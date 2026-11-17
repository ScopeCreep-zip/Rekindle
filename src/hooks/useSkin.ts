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

/** Apply `skin` to this window. */
export function applySkin(skin: SkinApplied): void {
  const root = document.documentElement;
  root.dataset.skinBase = skin.base;
  let link = document.getElementById(LINK_ID) as HTMLLinkElement | null;
  if (link === null) {
    link = document.createElement("link");
    link.id = LINK_ID;
    link.rel = "stylesheet";
    document.head.appendChild(link);
  }
  if (link.getAttribute("href") !== skin.tokensHref) {
    link.href = skin.tokensHref;
  }
}
