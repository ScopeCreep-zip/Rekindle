import { getCurrentWindow } from "@tauri-apps/api/window";
import { createSignal, onCleanup } from "solid-js";
import { subscribeSkinApplyFailed, subscribeSkinEvents } from "../ipc/channels";
import { commands } from "../ipc/commands";
import type { BrandGlyphName, SkinApplied, SkinApplyFailure } from "../ipc/commands/dto";
import type { SkinShell } from "../ipc/commands/dto_skin_tree";
import { Channel } from "../ipc/invoke";
import { addToast } from "../stores/toast.store";

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

/// How many sheets this window has linked: what reads the shown skin's
/// custom properties (a pack-served icon, plan SK17-7a) reads them again.
const [sheetsLinked, setSheetsLinked] = createSignal(0);

/** Bumps each time this window links a skin's sheet (its custom properties may have changed). */
export function skinSheetLinked(): number {
  return sheetsLinked();
}

/**
 * Set the shown skin's trees, keeping each one whose content is unchanged
 * as the same object: a skin re-applied unchanged (every window's
 * `SkinApplied`) then never re-creates a window's tree, whose slot mounts
 * would re-append in a new order (plan CR1 audit).
 */
function setShells(next: SkinShell[]): void {
  setSkinShells((shown) =>
    next.map((n) => {
      const same = shown.find((s) => s.window === n.window && s.popup?.name === n.popup?.name);
      return same && JSON.stringify(same) === JSON.stringify(n) ? same : n;
    })
  );
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
 * popup's, the buddy list, chat and community windows their window's own;
 * every other window none, so it decodes none of their art.
 */
function boundShells(shells: SkinShell[]): SkinShell[] {
  const path = window.location.pathname;
  if (path.startsWith("/popup/")) {
    const name = popupName();
    return shells.filter((s) => s.popup?.name === name);
  }
  // A community window with a community (`/community?id=`, plan SKW1) wears
  // the community frame; the community browser (no `id`) none.
  const kind = framedWindow();
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
  let received = false;
  const frame = framedWindow();
  if (frame) {
    // Hidden until this document's first committed skin (applySkin clears it
    // before the commit that lets native reveal the window); a deadline
    // failure shows the error instead.
    document.documentElement.dataset.skinOpening = "";
    void commands
      .waitSkinOpen()
      .then((error) => {
        if (error !== null && !stopped) showOpeningError(error);
      })
      .catch(logSkinError);
  }
  const apply = (skin: SkinApplied | null): void => {
    if (!openingFailed) void applySkin(skin, frame).catch(logSkinError);
  };
  const unsubscribe = subscribeSkinEvents((skin) => {
    received = true;
    apply(skin);
  });
  const unsubscribeFailure = subscribeSkinApplyFailed(({ window, cause, notice, reverted }) => {
    // The Host's words for it (SkinApplyFailure::notice), never the shell's.
    if (cause.type === "loadTimeout" && window === getCurrentWindow().label && !reverted) {
      // Native also reports a switch whose allocation or JS handler never started.
      showSkinError(notice);
    }
    if (cause.type === "windowTooSmall" && !document.getElementById(LINK_ID) && !loading) {
      addToast(notice);
    }
  });
  void commands
    .getSkin()
    .then((skin) => {
      if (!stopped && !received) apply(skin);
    })
    .catch(logSkinError);
  onCleanup(() => {
    stopped = true;
    pending?.abort();
    void unsubscribe.then((unlisten) => unlisten());
    void unsubscribeFailure.then((unlisten) => unlisten());
  });
}

function showOpeningError(message: string): void {
  openingFailed = true;
  pending?.abort();
  document.documentElement.dataset.skinOpening = "";
  showSkinError(message);
}

/**
 * Show `message` (the Host's words) in this window's skin alert: the whole
 * window while it opens, else a strip above the page, in flow, so the page
 * shrinks beneath it and no control is covered (22-skin-frame.css).
 */
function showSkinError(message: string): void {
  if (!skinError) {
    skinError = document.createElement("div");
    skinError.className = "skin-load-error";
    skinError.setAttribute("role", "alert");
    document.body.prepend(skinError);
  }
  skinError.textContent = message;
}

function logSkinError(error: unknown): void {
  console.error("Skin presentation failed", error);
}

let generation = 0;
let pending: AbortController | undefined;
let openingFailed = false;
let loading: HTMLLinkElement | null = null;
let skinError: HTMLDivElement | undefined;

function framedWindow(): SkinShell["window"] | undefined {
  const path = window.location.pathname;
  if (path === "/buddy-list") return "main";
  if (path.startsWith("/chat")) return "chat";
  if (path.startsWith("/community") && new URLSearchParams(window.location.search).has("id")) {
    return "community";
  }
  return undefined;
}

/**
 * Hold decoded images through the stylesheet swap. A native deadline
 * cancels the apply even if neither the image decoder nor the link settles.
 */
async function applySkin(skin: SkinApplied | null, frame: SkinShell["window"] | undefined) {
  // A framed document keeps its last committed skin during a native
  // allocation change. Its first open has its own native error surface.
  if (skin === null && frame) return;
  pending?.abort();
  const attempt = new AbortController();
  pending = attempt;
  const serial = ++generation;
  const shown = document.getElementById(LINK_ID);
  if (skin === null) {
    shown?.remove();
    delete document.documentElement.dataset.skinBase;
    setShells([]);
    setSkinTitleLogo(undefined);
    setSheetsLinked((n) => n + 1);
    return;
  }

  const expired = new Channel<string>((message) => {
    if (!attempt.signal.aborted) {
      attempt.abort();
      showSkinError(message);
    }
  });
  let attemptId: string | null = null;
  let href: string | null = null;
  try {
    attemptId = await commands.startSkinApply(serial, skin.id, skin.tokensHref, expired);
    if (attemptId === null || attempt.signal.aborted) return;
    await untilAborted(commitSkin(skin, shown, frame, attempt.signal), attempt.signal);
    if (!attempt.signal.aborted) {
      href = skin.tokensHref;
      skinError?.remove();
      skinError = undefined;
      // Displayable before the commit below, which is what lets native
      // reveal the window: its first shown frame is the skinned page.
      if (frame && !openingFailed) delete document.documentElement.dataset.skinOpening;
    }
  } catch (error) {
    if (attempt.signal.aborted) return;
    logSkinError(error);
    const notice = await report(
      skin,
      error instanceof SkinImageError
        ? { type: "imageDecode", data: { path: error.path } }
        : {
            type: "sheetError",
            data: { detail: error instanceof Error ? error.message : String(error) },
          }
    );
    if (
      notice !== null &&
      !attempt.signal.aborted &&
      document.documentElement.dataset.skinOpening === undefined
    ) {
      showSkinError(notice);
    }
  } finally {
    attempt.abort();
    if (attemptId !== null) await commands.finishSkinApply(attemptId, href);
  }
}

async function commitSkin(
  skin: SkinApplied,
  shown: HTMLElement | null,
  frame: SkinShell["window"] | undefined,
  signal: AbortSignal
): Promise<void> {
  const sameSheet = shown?.getAttribute("href") === skin.tokensHref;
  if (!sameSheet) {
    const hrefs = [
      ...new Set([
        ...skin.images,
        ...(frame ? (skin.frameImages?.[frame] ?? []) : []),
        ...boundShells(skin.shells ?? []).flatMap((s) => s.images),
      ]),
    ];
    const images = hrefs.map((href) => {
      const img = new Image();
      img.src = href;
      return img;
    });
    signal.addEventListener(
      "abort",
      () => {
        for (const img of images) img.removeAttribute("src");
      },
      { once: true }
    );
    await Promise.all(
      images.map((img, i) =>
        img.decode().catch(() => {
          throw new SkinImageError(hrefs[i]);
        })
      )
    );
    if (signal.aborted) return;
    const next = await linkSheet(skin.tokensHref, signal);
    if (signal.aborted) {
      next.remove();
      return;
    }
    shown?.remove();
    next.id = LINK_ID;
    images.length = 0;
  }
  document.documentElement.dataset.skinBase = skin.base;
  setShells(skin.shells ?? []);
  setSkinTitleLogo(skin.titleLogo);
  if (!sameSheet) setSheetsLinked((n) => n + 1);
  if (frame) {
    const hasTree = skin.shells?.some((s) => s.window === frame && !s.popup) ?? false;
    await waitForTree(frame, hasTree, signal);
  }
}

function untilAborted(work: Promise<void>, signal: AbortSignal): Promise<void> {
  return Promise.race([
    work,
    new Promise<void>((resolve) =>
      signal.addEventListener("abort", () => resolve(), { once: true })
    ),
  ]);
}

/** The route is lazy; wait until its frame and any served tree are in the DOM. */
function waitForTree(
  frame: SkinShell["window"],
  hasTree: boolean,
  signal: AbortSignal
): Promise<void> {
  const selector = `.app-frame[data-skin-window="${frame}"]${hasTree ? " .skin-tree" : ""}`;
  if (document.querySelector(selector)) return Promise.resolve();
  return new Promise((resolve) => {
    const observer = new MutationObserver(() => {
      if (document.querySelector(selector)) {
        observer.disconnect();
        resolve();
      }
    });
    observer.observe(document.body, { childList: true, subtree: true });
    signal.addEventListener("abort", () => observer.disconnect(), { once: true });
  });
}

class SkinImageError extends Error {
  constructor(readonly path: string) {
    super(`${path} failed to decode`);
  }
}

function linkSheet(href: string, signal: AbortSignal): Promise<HTMLLinkElement> {
  return new Promise((resolve, reject) => {
    const next = document.createElement("link");
    next.rel = "stylesheet";
    next.href = href;
    loading = next;
    const cancel = (): void => {
      loading = null;
      next.remove();
    };
    signal.addEventListener("abort", cancel, { once: true });
    next.addEventListener(
      "load",
      () => {
        loading = null;
        signal.removeEventListener("abort", cancel);
        resolve(next);
      },
      { once: true, signal }
    );
    next.addEventListener(
      "error",
      () => {
        cancel();
        reject(new Error(`${href} failed to load`));
      },
      { once: true, signal }
    );
    document.head.appendChild(next);
  });
}

/**
 * Tell the Host this window could not apply `skin` (plan SK2-T1). The
 * window only reports: the Host logs it, tells this window and Settings,
 * and reverts `skin` if it is on trial. It answers what the window tells
 * the user, in its words (SkinApplyFailure::notice); null if the report
 * did not reach it.
 */
async function report(skin: SkinApplied, cause: SkinApplyFailure): Promise<string | null> {
  try {
    return await commands.reportSkinApplyFailed(skin.id, cause);
  } catch (err: unknown) {
    console.error(`skin ${skin.id}: the failure was not reported`, err);
    return null;
  }
}
