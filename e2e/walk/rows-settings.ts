// Plan E7.13: the Settings rows (ST*) of
// .claude/plans/standards-remediation/evidence/app-journeys-2026-11-17.md,
// walked by the identity rows.ts created. Selectors come from
// windows/SettingsWindow.tsx and windows/settings/*.

import { NET_WAIT_MS, Skip, WAIT_MS, byText, present, resolveIntent, shown, waitUntil } from "./helpers.ts";
import { closeSettings, flipPersisted, inBuddy, openSettings, optionBox, pickNextFile, setValue, waitStored } from "./nav.ts";
import type { Row } from "./rows.ts";

/** A 1x1 PNG (the avatar upload's input accepts png/jpeg/gif up to 256 KB). */
const PNG_1X1 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

const ONE_SKIN = "one skin listed (Symbiosis: shown and the login skin); a second needs an imported skin package (ST11/ST12)";

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
    title: "Appearance: select skin, then ST13 keep",
    async run({ b, s, shot }) {
      await openSettings(b, "Appearance");
      await shown(b, ".skin-row", NET_WAIT_MS);
      const target = await b.$(".skin-row:not(.skin-row-shown) input[type=radio]");
      if (!(await target.isExisting())) {
        await shot();
        await closeSettings(b);
        throw new Skip(ONE_SKIN);
      }
      const name = await (await (await target.parentElement()).$(".skin-name")).getText();
      await target.click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await shown(b, ".skin-trial", NET_WAIT_MS);
      await shot();
      await (await b.$(".skin-trial .sec-btn-commit")).click();
      await waitUntil(async () => !(await present(b, ".skin-trial")), NET_WAIT_MS, "trial prompt stayed after Keep");
      const shownName = await (await b.$(".skin-row-shown .skin-name")).getText();
      if (shownName !== name) throw new Error(`kept ${name} but ${shownName} is shown`);
      await closeSettings(b);
      return `chose ${name}; the trial prompt showed; Keep kept it`;
    },
  },
  {
    id: "ST13",
    title: "Appearance: skin trial revert",
    async run({ b, s, shot }) {
      await openSettings(b, "Appearance");
      await shown(b, ".skin-row", NET_WAIT_MS);
      const kept = await (await b.$(".skin-row-shown .skin-name")).getText();
      if (!(await (await b.$(".skin-row:not(.skin-row-shown) input[type=radio]")).isExisting())) {
        await shot();
        await closeSettings(b);
        throw new Skip(ONE_SKIN);
      }
      await (await b.$(".skin-row:not(.skin-row-shown) input[type=radio]")).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await shown(b, ".skin-trial", NET_WAIT_MS);
      await (await b.$(".skin-trial .sec-btn-secondary")).click();
      await waitUntil(async () => !(await present(b, ".skin-trial")), NET_WAIT_MS, "trial prompt stayed after Revert");
      await waitUntil(async () => (await (await b.$(".skin-row-shown .skin-name")).getText()) === kept, WAIT_MS, "skin did not revert");
      await shot();
      await closeSettings(b);
      return `reverted to ${kept}`;
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
      const row = await btn.parentElement().then((el) => el.parentElement());
      const name = await (await row.$(".skin-name")).getText();
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
      if (!(await box.isSelected())) await box.click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      const nums = 'input.form-input[type="number"]';
      await waitUntil(async () => (await (await b.$(nums)).isEnabled()), WAIT_MS, "start hour stayed disabled");
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
      const [s2, e2] = await b.$$(nums);
      const got = `${await s2.getValue()}-${await e2.getValue()}`;
      const on = await (await optionBox(b, "Enable quiet hours")).isSelected();
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
