// Plan E7.14 S1: create an identity in the real app. The Host makes the
// keys, the vault and the database, the Veilid node attaches (Create stays
// disabled until it can), and the buddy list opens with the new name.

import { randomBytes } from "node:crypto";
import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, resolveIntent, shown, switchTo, typeInto, waitForHandle } from "../../walk/helpers.ts";
import { BUDDY, LOGIN, inBuddy } from "../../walk/nav.ts";

describe("identity", () => {
  it("creates an identity and opens the buddy list under its name", async () => {
    const displayName = `wdio-${randomBytes(3).toString("hex")}`;
    const passphrase = randomBytes(18).toString("base64url");

    await switchTo(browser, LOGIN, NET_WAIT_MS);
    // First run opens the create form without the account picker
    // (LoginWindow.tsx); the button is there only once identities exist.
    const create = await browser.$(".account-create-btn");
    if (await create.isDisplayed()) await create.click();
    await typeInto(await shown(browser, 'input[placeholder="Display Name (optional)"]', NET_WAIT_MS), displayName);
    await typeInto(await shown(browser, 'form.login-container input[type="password"]'), passphrase);

    const submit = await browser.$('form.login-container button[type="submit"]');
    await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Create Identity stayed disabled (Connecting…)" });
    await submit.click();
    await resolveIntent(browser, { passphrase });

    await waitForHandle(browser, (h) => h === BUDDY, NET_WAIT_MS);
    await inBuddy(browser);
    // The bar renders before the store hydrates ("Unknown" first, then the name).
    await expect(browser.$(".identity-bar-name")).toHaveText(expect.stringContaining(displayName), { wait: 10_000 });
  });
});
