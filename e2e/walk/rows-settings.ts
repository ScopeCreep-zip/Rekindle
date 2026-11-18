// Plan E7.13: the Settings rows (ST*) of
// .claude/plans/standards-remediation/evidence/app-journeys-2026-11-17.md,
// walked by the identity rows.ts created. Selectors come from
// windows/SettingsWindow.tsx and windows/settings/*.

import { NET_WAIT_MS, Skip, WAIT_MS, byText, present, resolveIntent, shown, waitUntil } from "./helpers.ts";
import { closeSettings, flipPersisted, inBuddy, openSettings, optionBox, pickNextFile, setValue, waitStored } from "./nav.ts";
import { readFileSync } from "node:fs";
import type { Browser } from "webdriverio";
import type { Row } from "./rows.ts";

/** A 1x1 PNG (the avatar upload's input accepts png/jpeg/gif up to 256 KB). */
const PNG_1X1 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

const ONE_SKIN = "one skin listed (Symbiosis: shown and the login skin); a second needs an imported skin package (ST11/ST12)";

/**
 * The first listed skin that is not the one shown, by its name, and its
 * radio; null when only one skin is listed.
 */
async function otherSkin(b: Browser): Promise<{ name: string; radio: Awaited<ReturnType<Browser["$"]>> } | null> {
  for (const row of await b.$$(".skin-row:not(.skin-row-shown)")) {
    const name = (await (await row.$(".skin-name")).getText()).trim();
    if (name) return { name, radio: await row.$("input[type=radio]") };
  }
  return null;
}

/**
 * What the Appearance tab shows when the trial prompt never does: the
 * shown skin, the checked radio and any refusal (AppearanceTab.tsx:164).
 */
async function skinProbe(b: Browser): Promise<string> {
  return b.execute(() => {
    const shownName = document.querySelector(".skin-row-shown .skin-name")?.textContent?.trim();
    const checked = (document.querySelector(".skin-row input[type=radio]:checked") as HTMLElement | null)
      ?.closest(".skin-row")
      ?.querySelector(".skin-name")?.textContent?.trim();
    const refusal = document.querySelector(".skin-error")?.textContent?.trim();
    const review = !!document.querySelector(".intent-dialog-actions");
    return `shown=${shownName}; checked=${checked}; refusal=${refusal ?? "none"}; review open=${review}`;
  });
}

const ONYX = new URL("../../crates/rekindle-skin/builtin/onyx.rkskin", import.meta.url);

type Radio = Awaited<ReturnType<Browser["$"]>>;

/** The name of the skin every window shows now. */
async function shownSkin(b: Browser): Promise<string> {
  return (await (await b.$(".skin-row-shown .skin-name")).getText()).trim();
}

/** Whether a row offers Remove, i.e. holds an imported (library) skin. */
async function offersRemove(row: Radio): Promise<boolean> {
  for (const action of await row.$$(".skin-action")) {
    if ((await action.getText()).trim() === "Remove") return true;
  }
  return false;
}

/** The radio of the built-in skin named `name`. */
async function rowRadio(b: Browser, name: string): Promise<Radio> {
  for (const row of await b.$$(".skin-row")) {
    if ((await (await row.$(".skin-name")).getText()).trim() === name && !(await offersRemove(row))) {
      return row.$("input[type=radio]");
    }
  }
  throw new Error(`no built-in skin named ${name}`);
}

/** The radio of the imported skin, or null when none is listed. */
async function importedRadio(b: Browser): Promise<Radio | null> {
  for (const row of await b.$$(".skin-row")) {
    if (await offersRemove(row)) return row.$("input[type=radio]");
  }
  return null;
}

/** Whether the shown skin is the imported one. */
async function importedShown(b: Browser): Promise<boolean> {
  const row = await b.$(".skin-row-shown");
  return (await row.isExisting()) && (await offersRemove(row));
}

const TABS = ["Profile", "Application", "Appearance", "Notifications", "Audio", "Video", "Privacy", "Devices", "Mobile", "About"];

function flipRow(id: string, tab: string, text: string): Row {
  return {
    id,
    title: `Settings > ${tab}: ${text}`,
    async run({ b, s, shot }) {
      await openSettings(b, tab);
      const note = await flipPersisted(b, tab, text, s.passphrase);
      await shot();
      await closeSettings(b);
      return note;
    },
  };
}

export const SETTINGS_ROWS: Row[] = [
  {
    id: "ST1",
    title: "Switch tabs",
    async run({ b, shot }) {
      await openSettings(b, "Profile");
      for (const tab of TABS) {
        await (await byText(b, ".form-tab", tab)).click();
        await waitUntil(async () => (await (await b.$(".form-tab-active")).getText()).trim() === tab, WAIT_MS, `${tab} did not activate`);
      }
      await shot();
      await closeSettings(b);
      return `activated ${TABS.length} tabs in turn`;
    },
  },
  {
    id: "ST2",
    title: "Profile: upload avatar",
    async run({ b, s, shot }) {
      await openSettings(b, "Profile");
      const before = await (await shown(b, ".avatar-upload-section")).getHTML();
      await pickNextFile(b, "walk.png", "image/png", PNG_1X1);
      await (await b.$(".avatar-upload-btn")).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await waitUntil(async () => (await (await b.$(".avatar-upload-section")).getHTML()) !== before, NET_WAIT_MS, "avatar did not change");
      await shot();
      await closeSettings(b);
      return "file chosen by stubbing the OS picker's input (WebDriver cannot drive the native dialog)";
    },
  },
  {
    id: "ST3",
    title: "Profile: save name (Enter)",
    async run({ b, s, shot }) {
      await openSettings(b, "Profile");
      const next = `${s.displayName}-st`;
      const input = await shown(b, '.settings-content input.form-input[type="text"]:not([placeholder])');
      await input.click();
      await input.clearValue();
      await input.addValue(next);
      await b.keys("Enter");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await shot();
      await closeSettings(b);
      await waitUntil(async () => (await (await b.$(".identity-bar-name")).getText()).includes(next), NET_WAIT_MS, "buddy list did not show the new name");
      s.displayName = next;
      return `buddy list shows ${next}`;
    },
  },
  {
    id: "ST4",
    title: "Profile: save status message (button)",
    async run({ b, s, shot }) {
      await openSettings(b, "Profile");
      const msg = "set from settings";
      const input = await shown(b, `input[placeholder="What's on your mind?"]`);
      await input.click();
      await input.clearValue();
      await input.addValue(msg);
      // The second Save on the tab (ProfileTab.tsx:94) is the status message's.
      const saves = await b.$$(".settings-content .form-field-row button.form-btn-primary");
      await saves[1].click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await shot();
      await closeSettings(b);
      await waitUntil(async () => (await (await b.$(".identity-bar-status-msg")).getText()).includes(msg), NET_WAIT_MS, "buddy list did not show the message");
      return "buddy list shows the message";
    },
  },
  {
    id: "ST5",
    title: "Application: auto-start",
    async run() {
      throw new Skip("writes the OS login items (commands/settings.rs:62 apply_auto_start), outside the walk's temp root");
    },
  },
  flipRow("ST6", "Application", "Start Minimized"),
  flipRow("ST7", "Application", "Show Game Activity"),
  {
    id: "ST8",
    title: "Application: auto-away minutes",
    async run({ b, shot }) {
      await openSettings(b, "Application");
      await setValue(b, ".settings-content select.form-select", "15");
      await waitStored(b, "autoAwayMinutes", 15);
      await closeSettings(b);
      await openSettings(b, "Application");
      const v = await (await b.$(".settings-content select.form-select")).getValue();
      await shot();
      if (v !== "15") throw new Error(`auto-away read back ${v}`);
      await setValue(b, ".settings-content select.form-select", "0");
      await waitStored(b, "autoAwayMinutes", 0);
      await closeSettings(b);
      return "15 minutes kept after reopen; set back to Disabled";
    },
  },
  {
    id: "ST9",
    title: "Appearance: a built-in skin is chosen at once",
    async run({ b, s, shot }) {
      // Only a non-built-in skin goes on trial (plan E7.7.2 "Apply";
      // host/skin/mod.rs select); walk 22's probe after choosing the
      // built-in Xfire read "shown=Xfire; checked=Xfire; refusal=none".
      await openSettings(b, "Appearance");
      await shown(b, ".skin-row", NET_WAIT_MS);
      const first = await (await b.$(".skin-row-shown .skin-name")).getText();
      const other = await otherSkin(b);
      if (!other) {
        await shot();
        await closeSettings(b);
        throw new Skip(ONE_SKIN);
      }
      await other.radio.click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await waitUntil(async () => (await shownSkin(b)) === other.name, WAIT_MS, `${other.name} not shown`);
      if (await present(b, ".skin-trial")) throw new Error(`a built-in went on trial (${await skinProbe(b)})`);
      await closeSettings(b);
      await openSettings(b, "Appearance");
      await waitUntil(async () => (await shownSkin(b)) === other.name, WAIT_MS, `${other.name} not kept after reopen`);
      await shot();
      await (await rowRadio(b, first)).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await waitUntil(async () => (await shownSkin(b)) === first, WAIT_MS, `${first} not shown again`);
      await closeSettings(b);
      return `${other.name} stored at once with no trial, kept after reopen; set back to ${first}`;
    },
  },
  {
    id: "ST13",
    title: "Appearance: an imported skin's trial (Revert, then Keep)",
    async run({ b, s, shot }) {
      await openSettings(b, "Appearance");
      await shown(b, ".skin-row", NET_WAIT_MS);
      const kept = await shownSkin(b);
      // A real package: the Onyx built-in's bytes, imported as a library
      // skin (pkg:<hash>), whose row offers Remove (AppearanceTab.tsx).
      await pickNextFile(b, "walk-onyx.rkskin", "application/zip", readFileSync(ONYX).toString("base64"));
      await (await byText(b, ".skin-import button", "Import skin")).click();
      if ((await resolveIntent(b, { passphrase: s.passphrase })) !== "committed") throw new Error("import showed no review");
      const trialOf = async (answer: string): Promise<void> => {
        await waitUntil(async () => (await importedRadio(b)) !== null, NET_WAIT_MS, "no imported skin listed");
        const radio = await importedRadio(b);
        if (!radio) throw new Error("the imported skin left the list");
        await radio.click();
        await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
        await shown(b, ".skin-trial", NET_WAIT_MS).catch(async (e: Error) => {
          throw new Error(`${e.message} after choosing the imported skin (${await skinProbe(b)})`);
        });
        await (await b.$(`.skin-trial ${answer}`)).click();
        await waitUntil(async () => !(await present(b, ".skin-trial")), NET_WAIT_MS, "trial prompt stayed after the answer");
      };
      await trialOf(".sec-btn-secondary");
      await waitUntil(async () => (await shownSkin(b)) === kept, WAIT_MS, "Revert did not restore the skin");
      await trialOf(".sec-btn-commit");
      await waitUntil(async () => (await importedShown(b)), WAIT_MS, "Keep did not keep the imported skin");
      await shot();
      await (await rowRadio(b, kept)).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await waitUntil(async () => (await shownSkin(b)) === kept, WAIT_MS, `${kept} not shown again`);
      await closeSettings(b);
      return `Revert restored ${kept}; Keep kept the imported skin; set back to ${kept}`;
    },
  },
  {
    id: "ST10",
    title: "Appearance: set login skin",
    async run({ b, s, shot }) {
      await openSettings(b, "Appearance");
      await shown(b, ".skin-row", NET_WAIT_MS);
      if (!(await present(b, ".skin-action"))) {
        await shot();
        await closeSettings(b);
        throw new Skip(ONE_SKIN);
      }
      const btn = await byText(b, ".skin-action", "Use as login skin");
      // The row's name from the page: webdriverio cannot walk up from an
      // element `$$` returned (walk 21 ST10: "parent element ... wasn't found").
      const name = await b.execute(() => {
        const use = Array.from(document.querySelectorAll(".skin-action")).find((x) => x.textContent?.trim() === "Use as login skin");
        return use?.closest(".skin-row")?.querySelector(".skin-name")?.textContent?.trim() ?? "";
      });
      await btn.click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await waitUntil(async () => (await (await b.$(".skin-badge")).parentElement().then((r) => r.$(".skin-name")).then((n) => n.getText())) === name, WAIT_MS, "Login skin badge did not move");
      await shot();
      await closeSettings(b);
      return `login skin badge on ${name}`;
    },
  },
  {
    id: "ST11",
    title: "Appearance: remove skin",
    async run() {
      throw new Skip("Remove shows only for an imported skin (AppearanceTab.tsx:102); ST12 has no valid skin package to import");
    },
  },
  {
    id: "ST12",
    title: "Appearance: import skin (a non-skin file is refused)",
    async run({ b, s, shot }) {
      await openSettings(b, "Appearance");
      await shown(b, ".skin-row", NET_WAIT_MS);
      await pickNextFile(b, "not-a-skin.zip", "application/zip", btoa("not a skin"));
      await (await byText(b, ".skin-import button", "Import skin")).click();
      // ImportSkin is TrustChanging (intent/kind.rs:438): the Host holds the
      // review first and checks the package after the commit
      // (host/skin_tests.rs:96, review then the helper's 400).
      if ((await resolveIntent(b, { passphrase: s.passphrase })) !== "committed") throw new Error("import showed no review");
      const err = await shown(b, ".skin-error", NET_WAIT_MS);
      const text = await err.getText();
      await shot();
      await closeSettings(b);
      // The helper's check must be what refused it (walk 6 read "helper
      // could not start" here before the walk built the helper).
      if (text.includes("could not start")) throw new Error(`the helper did not run: ${text.slice(0, 160)}`);
      return `refused: ${text.slice(0, 120)}`;
    },
  },
  flipRow("ST14", "Notifications", "Enable Notifications"),
  flipRow("ST15", "Notifications", "Sound Effects"),
  flipRow("ST16", "Notifications", "Play ringtone on incoming calls"),
  {
    id: "ST17",
    title: "Notifications: volume slider",
    async run({ b, shot }) {
      await openSettings(b, "Notifications");
      await setValue(b, '.settings-content input[type="range"]', "0.35");
      await byText(b, "label.settings-option", "35%");
      await closeSettings(b);
      await openSettings(b, "Notifications");
      await byText(b, "label.settings-option", "35%", WAIT_MS);
      await shot();
      await closeSettings(b);
      return "35% kept after reopen";
    },
  },
  flipRow("ST18", "Notifications", "Suppress message notifications while in a call"),
  {
    id: "ST19",
    title: 'Notifications: "Notifications show"',
    async run({ b, shot }) {
      await openSettings(b, "Notifications");
      // The tab has two selects (NotificationsTab.tsx:155 and the quiet-hours
      // timezone at :249): "Notifications show" is the one offering nameOnly.
      const sel = '.settings-content select.form-select:has(option[value="nameOnly"])';
      await setValue(b, sel, "nameOnly");
      await waitStored(b, "notificationContent", "nameOnly");
      await closeSettings(b);
      await openSettings(b, "Notifications");
      const v = await (await b.$(sel)).getValue();
      await shot();
      if (v !== "nameOnly") throw new Error(`read back ${v}`);
      await setValue(b, sel, "nameAndMessage");
      await waitStored(b, "notificationContent", "nameAndMessage");
      await closeSettings(b);
      return "Name only kept after reopen; set back";
    },
  },
  flipRow("ST20", "Notifications", "Suppress all notifications"),
  flipRow("ST21", "Notifications", "Generate previews"),
  {
    id: "ST22",
    title: "Quiet hours: enable, start, end",
    async run({ b, s, shot }) {
      await openSettings(b, "Notifications");
      const box = await optionBox(b, "Enable quiet hours");
      // Enabled once the stored hours have loaded (NotificationsTab.tsx).
      await box.waitForEnabled({ timeout: WAIT_MS, timeoutMsg: "quiet-hours box stayed disabled" });
      if (!(await box.isSelected())) await box.click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      const nums = 'input.form-input[type="number"]';
      // Read the page itself, not a driver element: `waitUntil` turns a
      // thrown lookup into `false`, so the old wait said "stayed disabled"
      // for any failure, while walk 8's ST22.png shows the box ticked and
      // the field enabled. A timeout now names what the page held.
      let seen = "not read";
      await waitUntil(
        async () => {
          seen = await b.execute((sel: string) => {
            const box = Array.from(document.querySelectorAll("label.settings-option"))
              .find((l) => l.textContent?.includes("Enable quiet hours"))
              ?.querySelector<HTMLInputElement>("input[type=checkbox]");
            const field = document.querySelector<HTMLInputElement>(sel);
            const boxState = box ? `${box.disabled ? "disabled," : ""}${box.checked ? "ticked" : "clear"}` : "missing";
            const fieldState = field ? (field.disabled ? "disabled" : "enabled") : "missing";
            return `box=${boxState} start=${fieldState}`;
          }, nums);
          return seen.endsWith("start=enabled");
        },
        WAIT_MS,
        "start hour not enabled",
      ).catch((e: Error) => {
        throw new Error(`${e.message} (${seen})`);
      });
      const [start, end] = await b.$$(nums);
      for (const [el, v] of [[start, "22"], [end, "7"]] as const) {
        await b.execute((n: HTMLInputElement, x: string) => {
          n.value = x;
          n.dispatchEvent(new Event("change", { bubbles: true }));
        }, el as unknown as HTMLInputElement, v);
        await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      }
      await closeSettings(b);
      await openSettings(b, "Notifications");
      // The unloaded defaults (off, 22-7) are what the row writes, so read
      // back only once the stored values have loaded (the box is enabled
      // then, NotificationsTab.tsx `qhLoaded`).
      const box2 = await optionBox(b, "Enable quiet hours");
      await box2.waitForEnabled({ timeout: WAIT_MS, timeoutMsg: "quiet-hours box stayed disabled on reopen" });
      const [s2, e2] = await b.$$(nums);
      const got = `${await s2.getValue()}-${await e2.getValue()}`;
      const on = await box2.isSelected();
      await shot();
      await closeSettings(b);
      if (!on || got !== "22-7") throw new Error(`read back enabled=${on} hours=${got}`);
      return "enabled 22-7, kept after reopen (timezone left at its default)";
    },
  },
  {
    id: "ST23",
    title: "Audio: input device",
    async run() {
      throw new Skip("[MIC]: opening an input device needs the microphone");
    },
  },
  {
    id: "ST24",
    title: "Audio: input channels",
    async run({ b, shot }) {
      await openSettings(b, "Audio");
      await sleepFor(1500);
      if (!(await present(b, ".settings-content label.settings-option input[type=checkbox]"))) {
        await shot();
        await closeSettings(b);
        throw new Skip("no input channel boxes: the default input reports one channel or none (AudioTab.tsx:95)");
      }
      const box = await b.$(".settings-content label.settings-option input[type=checkbox]");
      const before = await box.isSelected();
      await box.click();
      await waitUntil(async () => (await box.isSelected()) !== before, WAIT_MS, "input channel did not flip");
      await shot();
      await box.click();
      await closeSettings(b);
      return "flipped Input 1 and back";
    },
  },
  {
    id: "ST25",
    title: "Audio: output device",
    async run({ b, shot }) {
      await openSettings(b, "Audio");
      await sleepFor(1500);
      const selects = await b.$$(".settings-content select.form-select");
      const out = selects[selects.length - 1];
      const values = await out.$$("option").map((o) => o.getAttribute("value"));
      await shot();
      if (values.length < 2) {
        await closeSettings(b);
        throw new Skip(`output select lists ${values.length} option(s); nothing to change to`);
      }
      const before = await out.getValue();
      const next = values.find((v) => v !== before) ?? "";
      await b.execute((n: HTMLSelectElement, v: string) => {
        n.value = v;
        n.dispatchEvent(new Event("change", { bubbles: true }));
      }, out as unknown as HTMLSelectElement, next);
      await closeSettings(b);
      await openSettings(b, "Audio");
      await sleepFor(1500);
      const again = await b.$$(".settings-content select.form-select");
      const got = await again[again.length - 1].getValue();
      await closeSettings(b);
      if (got !== next) throw new Error(`output read back "${got}", chose "${next}"`);
      return `output ${before || "(default)"} -> ${next}, kept after reopen`;
    },
  },
  {
    id: "ST26",
    title: "Video: camera select",
    async run() {
      throw new Skip("[CAM]");
    },
  },
  {
    id: "ST27",
    title: "Video: refresh cameras",
    async run() {
      throw new Skip("[CAM]: getUserMedia");
    },
  },
  flipRow("ST28", "Privacy", "Read receipts"),
  {
    id: "ST29",
    title: "Privacy: unblock user",
    async run({ b, shot }) {
      await openSettings(b, "Privacy");
      await byText(b, ".settings-hint", "No blocked users.");
      await shot();
      await closeSettings(b);
      throw new Skip("no blocked user without a peer; the list shows No blocked users.");
    },
  },
  flipRow("ST30", "Privacy", "Auto-volunteer"),
  {
    id: "ST31",
    title: "Relay: revoke",
    async run() {
      throw new Skip("[PEER]: revoke lists only friends this identity relays for (RelaySettingsSection.tsx:68)");
    },
  },
  {
    id: "ST33",
    title: "Devices: list",
    async run({ b, shot }) {
      await openSettings(b, "Devices");
      await byText(b, ".settings-hint", "No paired devices yet.", NET_WAIT_MS);
      await shot();
      await closeSettings(b);
      return "No paired devices yet.";
    },
  },
  {
    id: "ST34",
    title: "Add device: generate QR (and ST35 copy URI)",
    async run({ b, s, shot }) {
      await openSettings(b, "Devices");
      await (await byText(b, ".settings-button-row .form-btn-primary", "Add a device")).click();
      await (await shown(b, ".add-device-stage .sec-btn-commit")).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await shown(b, '[aria-label="Pairing QR code"]', NET_WAIT_MS);
      const code = await (await shown(b, ".sec-pairing-code")).getText();
      await shot();
      await closeSettings(b);
      return `QR shown with a ${code.trim().split(/\s+/).length}-word code`;
    },
  },
  {
    id: "ST41",
    title: "About: check for updates",
    async run({ b, shot }) {
      await openSettings(b, "About");
      await (await byText(b, ".update-check-row button", "Check for Updates")).click();
      const status = await shown(b, ".update-status, .update-status-available", NET_WAIT_MS);
      const text = await status.getText();
      await shot();
      await closeSettings(b);
      await inBuddy(b);
      return text;
    },
  },
];

function sleepFor(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
