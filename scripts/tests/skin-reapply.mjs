// Run with node --experimental-transform-types --test scripts/tests/skin-reapply.mjs.
// Exercise useSkin with controllable native and stylesheet events.
import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { test } from "node:test";

const hook = new URL("../../src/hooks/useSkin.ts", import.meta.url);
const fixture = {
  decodeImage: () => Promise.resolve(),
  links: [],
  alerts: [],
  finishes: [],
  toasts: [],
};

const imports = {
  "@tauri-apps/api/window": `export const getCurrentWindow = () => ({ label: fixture.label });`,
  "solid-js": `
    export const createSignal = (value) => [() => value, (next) => {
      value = typeof next === "function" ? next(value) : next;
    }];
    export const onCleanup = () => {};
  `,
  "../ipc/channels": `
    export async function subscribeSkinEvents(apply) { fixture.apply = apply; }
    export async function subscribeSkinApplyFailed(fail) { fixture.fail = fail; }
  `,
  "../ipc/commands": `
    export const commands = {
      getSkin: () => new Promise(() => {}),
      waitSkinOpen: () => new Promise(() => {}),
      async startSkinApply(serial, _, href, expired) {
        fixture.startedHref = href;
        fixture.expire = expired.receive;
        return String(serial);
      },
      async finishSkinApply(id, href) { fixture.finishes.push([id, href]); },
    };
  `,
  "../ipc/invoke": `export class Channel { constructor(receive) { this.receive = receive; } }`,
  "../stores/toast.store": `export const addToast = (message) => fixture.toasts.push(message);`,
};
globalThis.fixture = fixture;
registerHooks({
  resolve(specifier, context, next) {
    if (context.parentURL?.split("?")[0] === hook.href && Object.hasOwn(imports, specifier)) {
      return {
        url: `data:text/javascript,${encodeURIComponent(imports[specifier])}`,
        shortCircuit: true,
      };
    }
    return next(specifier, context);
  },
});

class Element extends EventTarget {
  id = "";
  href = "";
  attributes = new Map();
  getAttribute(name) {
    return name === "href" ? this.href : this.attributes.get(name);
  }
  setAttribute(name, value) {
    this.attributes.set(name, value);
  }
  remove() {
    this.removed = true;
  }
}

globalThis.window = { location: { pathname: "/profile", search: "?key=friend" } };
globalThis.document = {
  documentElement: { dataset: {} },
  getElementById: (id) => fixture.links.find((link) => link.id === id && !link.removed) ?? null,
  querySelector: (selector) => selector === '.app-frame[data-skin-window="main"]',
  createElement: () => new Element(),
  head: { appendChild: (link) => fixture.links.push(link) },
  // The alert goes before the page (prepend), in flow above it, so it
  // covers no control; an alert appended over the page fails here.
  body: { prepend: (alert) => fixture.alerts.push(alert) },
};
globalThis.Image = class {
  decode() {
    return fixture.decodeImage(this.src);
  }
  removeAttribute() {
    this.src = "";
  }
};
async function openDocument(name, label = "profile") {
  fixture.links.length = 0;
  fixture.alerts.length = 0;
  fixture.finishes.length = 0;
  fixture.toasts.length = 0;
  fixture.decodeImage = () => Promise.resolve();
  fixture.label = label;
  window.location.pathname = `/${label}`;
  delete document.documentElement.dataset.skinOpening;
  const { useSkin } = await import(`${hook.href}?${name}`);
  useSkin();
  await apply(oldSkin);
}

const oldSkin = {
  id: "builtin:symbiosis",
  tokensHref: "skin://old/tokens.css",
  base: "dark",
  images: [],
};
const nextSkin = {
  ...oldSkin,
  id: "builtin:symbiosis-access",
  tokensHref: "skin://new/tokens.css",
};
const flush = () => new Promise(setImmediate);
const committed = () => document.getElementById("rekindle-skin")?.href;

async function apply(skin) {
  fixture.apply(skin);
  await flush();
  fixture.links.at(-1).dispatchEvent(new Event("load"));
  await flush();
}

test("a background profile can commit a skin while unrelated frame art stays pending", async () => {
  await openDocument("preloads");
  const frameArt = "skin://new/frame.png";
  const sharedArt = "skin://new/shared.png";
  const requested = [];
  fixture.decodeImage = (href) => {
    requested.push(href);
    return href === frameArt ? new Promise(() => {}) : Promise.resolve();
  };
  await apply({ ...nextSkin, images: [sharedArt], frameImages: { main: [frameArt] } });
  assert.deepEqual(requested, [sharedArt]);
  assert.equal(committed(), nextSkin.tokensHref);
  assert.deepEqual(fixture.finishes.at(-1), ["2", nextSkin.tokensHref]);
});

test("a failed switch stays visible on an open profile until a skin commits", async () => {
  await openDocument("watchdog");
  fixture.apply(nextSkin);
  await flush();
  const stalled = fixture.links.at(-1);
  const message = "Rekindle could not load the skin for this window in time.";
  fixture.expire(message);
  await flush();

  assert.equal(committed(), oldSkin.tokensHref);
  assert.deepEqual(fixture.finishes.at(-1), ["2", null]);
  assert.equal(document.documentElement.dataset.skinOpening, undefined);
  assert.equal(fixture.alerts.length, 1);
  const [alert] = fixture.alerts;
  assert.equal(alert.className, "skin-load-error");
  assert.equal(alert.getAttribute("role"), "alert");
  assert.equal(alert.textContent, message);

  stalled.dispatchEvent(new Event("load"));
  await flush();
  assert.equal(committed(), oldSkin.tokensHref);
  assert.equal(alert.removed, undefined);

  const later = { ...oldSkin, tokensHref: "skin://later/tokens.css" };
  fixture.apply(later);
  await flush();
  assert.equal(alert.removed, undefined);
  fixture.links.at(-1).dispatchEvent(new Event("load"));
  await flush();
  assert.equal(committed(), later.tokensHref);
  assert.equal(alert.removed, true);
});

test("an occluded buddy list swaps its sheet without animation frames", async () => {
  await openDocument("covered", "buddy-list");
  document.visibilityState = "hidden";
  let frameRequests = 0;
  globalThis.requestAnimationFrame = window.requestAnimationFrame = () => ++frameRequests;
  try {
    await apply({ ...nextSkin, frameImages: { main: ["skin://new/frame.png"] } });
    assert.equal(frameRequests, 0);
    assert.equal(fixture.startedHref, nextSkin.tokensHref);
    assert.equal(committed(), nextSkin.tokensHref);
    assert.deepEqual(fixture.finishes.at(-1), ["2", nextSkin.tokensHref]);
  } finally {
    delete document.visibilityState;
    delete window.requestAnimationFrame;
    delete globalThis.requestAnimationFrame;
  }
});

test("a buddy list shows its delivery timeout even if it never received the apply", async () => {
  await openDocument("delivery-timeout", "buddy-list");
  const failure = {
    cause: { type: "loadTimeout" },
    skin: nextSkin.id,
    notice: "Rekindle could not load the skin for this window in time.",
    reverted: false,
  };
  fixture.fail({ ...failure, window: "settings" });
  assert.equal(fixture.alerts.length, 0);
  fixture.fail({ ...failure, window: "buddy-list" });
  assert.equal(committed(), oldSkin.tokensHref);
  assert.equal(fixture.finishes.length, 1);
  assert.equal(fixture.alerts[0]?.textContent, failure.notice);
});
