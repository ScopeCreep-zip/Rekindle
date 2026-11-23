import { createSignal, onCleanup } from "solid-js";
import { commands } from "../ipc/commands";
import { subscribeSkinEvents } from "../ipc/channels";
import type { BrandGlyphName, SkinApplied, SkinApplyFailure } from "../ipc/commands/dto";
import type { SkinShell } from "../ipc/commands/dto_skin_tree";

/// The shown skin's window outlines (plan SK2-S2): set with the sheet that
/// places their nodes, so the nested divs and their rules always belong to
/// the same skin.
const [skinShells, setSkinShells] = createSignal<SkinShell[]>([]);
/// The mark the shown skin's title strip draws (plan BR), set with its sheet.
const [skinTitleLogo, setSkinTitleLogo] = createSignal<BrandGlyphName | undefined>();

/** The Rekindle mark the shown skin's title strip draws, if its strip has a logo. */
export function titleLogo(): BrandGlyphName | undefined {
  return skinTitleLogo();
}

/** The shown skin's outline for `window`, if it lays that window as a tree. */
export function skinShell(window: SkinShell["window"]): SkinShell | undefined {
  return skinShells().find((s) => s.window === window && s.popup === undefined);
}

/** The shown skin's tree for its popup `name` (plan SK2-P1b), if it lays one. */
export function skinPopupShell(name: string): SkinShell | undefined {
  return skinShells().find((s) => s.popup?.name === name);
}

/** The popup this window shows: `/popup/<name>`. */
export function popupName(): string {
  return decodeURIComponent(window.location.pathname.replace(/^\/popup\//, ""));
}

/**
 * The trees of `shells` this window lays (plan SK4): a popup window its
 * popup's, the buddy list and chat windows their window's own; every other
 * window none, so it decodes none of their art.
 */
function boundShells(shells: SkinShell[]): SkinShell[] {
  const path = window.location.pathname;
  if (path.startsWith("/popup/")) {
    const name = popupName();
    return shells.filter((s) => s.popup?.name === name);
  }
  const kind: SkinShell["window"] | undefined = path === "/buddy-list" ? "main" : path.startsWith("/chat") ? "chat" : undefined;
  return kind ? shells.filter((s) => s.window === kind && s.popup === undefined) : [];
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
 * whole. First every image it draws is decoded (`skin.images`, the shared
 * sheet's, plus its own trees' `images`, plan SK4): a
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
  // The shared sheet's images and this window's own trees' (plan SK4): a
  // window never waits on art another window's tree draws.
  const hrefs = [...new Set([...skin.images, ...boundShells(skin.shells ?? []).flatMap((s) => s.images)])];
  const loads = hrefs.map((href) => {
    const img = new Image();
    img.src = href;
    return { href, img };
  });
  const images = loads.map((l) => l.img);
  // Each decode names its own href, so the report says which image failed.
  const decoded = loads.map(({ href, img }) =>
    img.decode().catch((err: unknown) => {
      throw new SkinImageError(href, err);
    }),
  );
  Promise.all(decoded).then(
    () => {
      if (mine === generation) linkSheet(skin, mine, images);
    },
    (err: SkinImageError) => {
      if (mine !== generation) return;
      console.error(`skin ${skin.id}: ${err.path} failed to decode — the shown skin stays`, err.reason);
      report(skin, { type: "imageDecode", data: { path: err.path } });
    },
  );
}

/** A skin image that did not decode: its `skin://` href and the reason. */
class SkinImageError extends Error {
  constructor(
    readonly path: string,
    readonly reason: unknown,
  ) {
    super(`${path} failed to decode`);
  }
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
    setSkinTitleLogo(skin.titleLogo);
    images.length = 0;
  });
  next.addEventListener("error", () => {
    if (mine !== generation) return;
    loading = null;
    next.remove();
    console.error(`skin sheet ${skin.tokensHref} failed to load — the shown skin stays`);
    report(skin, { type: "sheetError", data: { detail: `${skin.tokensHref} failed to load` } });
  });
  next.href = skin.tokensHref;
  document.head.appendChild(next);
}

/**
 * Tell the Host this window could not apply `skin` (plan SK2-T1). The
 * window only reports: the Host logs it, tells this window and Settings,
 * and reverts `skin` if it is on trial.
 */
function report(skin: SkinApplied, cause: SkinApplyFailure): void {
  void commands
    .reportSkinApplyFailed(skin.id, cause)
    .catch((err: unknown) => console.error(`skin ${skin.id}: the failure was not reported`, err));
}
