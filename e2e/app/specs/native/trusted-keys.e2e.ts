import { browser, expect } from "@wdio/globals";
import { Key } from "webdriverio";
import { shot } from "../../shots.ts";

async function probeState() {
  return browser.execute(() => {
    const probe = document.getElementById("native-key-probe");
    return {
      focus: document.activeElement?.id,
      activations: Number(probe?.dataset.activations),
      events: JSON.parse(probe?.dataset.events ?? "[]"),
    };
  });
}

async function clearEvents() {
  await browser.execute(() => {
    const probe = document.getElementById("native-key-probe");
    if (probe) probe.dataset.events = "[]";
  });
}

async function expectKey(key: string, trusted: boolean) {
  const { events } = await probeState();
  expect(events).toEqual(
    expect.arrayContaining([
      { type: "keydown", key, trusted },
      { type: "keyup", key, trusted },
    ]),
  );
}

describe("Native WebDriver keys", function () {
  before(async function () {
    if (process.platform !== "darwin" && process.platform !== "linux") this.skip();

    await browser.execute(() => {
      const probe = document.createElement("section");
      probe.id = "native-key-probe";
      probe.dataset.events = "[]";
      probe.dataset.activations = "0";
      Object.assign(probe.style, {
        position: "fixed",
        top: "64px",
        left: "16px",
        zIndex: "2147483647",
        display: "grid",
        gap: "8px",
        padding: "16px",
        color: "black",
        background: "white",
      });
      const first = document.createElement("button");
      first.type = "button";
      first.id = "native-key-first";
      first.textContent = "First control · 0 activations";
      const second = document.createElement("button");
      second.type = "button";
      second.id = "native-key-second";
      second.textContent = "Second control";
      const scripted = document.createElement("input");
      scripted.id = "native-key-scripted";
      scripted.setAttribute("aria-label", "Scripted key control");
      scripted.placeholder = "Scripted key control";
      for (const control of [first, second, scripted]) {
        control.style.outlineOffset = "2px";
        control.addEventListener("focus", () => {
          control.style.outline = "2px solid #195cca";
        });
        control.addEventListener("blur", () => {
          control.style.outline = "";
        });
      }
      for (const type of ["keydown", "keyup"] as const) {
        probe.addEventListener(type, (event) => {
          const events = JSON.parse(probe.dataset.events ?? "[]");
          events.push({ type, key: event.key, trusted: event.isTrusted });
          probe.dataset.events = JSON.stringify(events);
        });
      }
      first.addEventListener("click", () => {
        const activations = Number(probe.dataset.activations) + 1;
        probe.dataset.activations = String(activations);
        first.textContent = `First control · ${activations} activations`;
      });
      probe.append(first, second, scripted);
      document.body.append(probe);
    });
  });

  after(async () => {
    await browser.execute(() => document.getElementById("native-key-probe")?.remove());
  });

  it("preserves the scripted element input as a negative control", async () => {
    await browser.$("#native-key-scripted").addValue(Key.Tab);
    await expectKey("Tab", false);
    expect((await probeState()).focus).toBe("native-key-scripted");
    await shot("scripted-tab-stays-in-input");
  });

  it("moves Linux focus and activates the focused button with trusted keys", async () => {
    if (process.platform === "linux") {
      await browser.execute(() => document.getElementById("native-key-first")?.focus());
      await clearEvents();
      await browser.keys(Key.Tab);
      await expectKey("Tab", true);
      expect((await probeState()).focus).toBe("native-key-second");
      await shot("native-tab-focuses-second-control");

      await clearEvents();
      await browser.action("key").down(Key.Shift).down(Key.Tab).up(Key.Tab).up(Key.Shift).perform();
      await expectKey("Tab", true);
      expect((await probeState()).focus).toBe("native-key-first");
      await shot("native-shift-tab-focuses-first-control");
    } else {
      await browser.execute(() => document.getElementById("native-key-first")?.focus());
    }

    await clearEvents();
    await browser.keys(Key.Enter);
    await expectKey("Enter", true);
    expect(await probeState()).toMatchObject({ focus: "native-key-first", activations: 1 });
    await shot("native-enter-activates-first-control");

    await clearEvents();
    await browser.keys(" ");
    await expectKey(" ", true);
    expect(await probeState()).toMatchObject({ focus: "native-key-first", activations: 2 });
    await shot("native-space-activates-first-control");
  });
});
