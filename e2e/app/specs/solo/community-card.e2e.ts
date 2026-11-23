// Plan E7.14 for E3.4 S6/S6b (round 24): our card in a community is the
// Host's and survives a restart. We set our name in the community (Settings →
// Overview, through the Host's review), and our bio and custom status (our
// profile's edit form); after logging off and unlocking again (the Host
// restores our card at unlock), the member list shows that name and status
// and our profile shows the bio. The community never shows our identity name.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, resolveIntent, shown, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { createCommunity, createIdentity, logOut, newUser, openCommunity, unlock, type User } from "../../journeys.ts";

const TOWN = "Card Town";
const NAME = "Bea in Card Town";
const BIO = "Here for the Friday matches.";
const STATUS = "afk for dinner";

/** The member row whose name reads `name`. */
async function memberRow(name: string): Promise<Awaited<ReturnType<typeof browser.$>>> {
  return byText(browser, ".member-item", name, NET_WAIT_MS);
}

/** An edit field in our profile's form, by its label's words. */
async function editField(label: string): Promise<Awaited<ReturnType<typeof browser.$>>> {
  const field = (await byText(browser, ".profile-popup-edit-label", label)).$("input, textarea");
  return field;
}

describe("our card in a community (E3.4 S6)", function () {
  this.timeout(900_000);
  const me: User = newUser("card");

  it("keeps our community name, bio and status across a restart", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, TOWN);
    await openCommunity(browser, TOWN);

    // Our name here. The Host classes it Routine (IntentKind::SetCommunityDisplayName,
    // rekindle-types intent/kind.rs base_class), so it commits with no review.
    await press(browser, await shown(browser, '[title="Community Settings"]'));
    await press(browser, await byText(browser, ".modal-container .form-tab", "Overview"));
    await typeInto(await shown(browser, ".my-name-section input"), NAME);
    await press(browser, await shown(browser, '.my-name-section button[type="submit"]'));
    expect(await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 5000 })).toBe("none");
    await byText(browser, ".my-name-section .settings-hint", `see you as ${NAME}`, NET_WAIT_MS);
    await browser.keys("Escape");

    // Our bio and custom status, from our own profile.
    await press(browser, await memberRow(NAME));
    await press(browser, await byText(browser, ".profile-popup-action-btn", "Edit profile"));
    await typeInto(await editField("Bio"), BIO);
    await typeInto(await editField("Custom status"), STATUS);
    await press(browser, await shown(browser, '.profile-popup-edit-actions button[type="submit"]'));
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 3000 });
    await byText(browser, ".member-item .member-custom-status", STATUS, NET_WAIT_MS);
    await browser.keys("Escape");
    await browser.closeWindow();

    // A restart of the session: the Host restores our card at unlock.
    await logOut(browser, me);
    await unlock(browser, me);
    await openCommunity(browser, TOWN);
    const row = await memberRow(NAME);
    expect(await row.getText()).toContain(STATUS);
    // The community never names us by our identity name.
    const names = (await browser.execute(() =>
      Array.from(document.querySelectorAll(".member-name")).map((n) => (n.textContent ?? "").trim()),
    )) as string[];
    expect(names).not.toContain(me.displayName);
    await press(browser, row);
    await waitUntil(
      async () => ((await (await browser.$(".profile-popup-bio")).getText()) ?? "").includes(BIO),
      NET_WAIT_MS,
      "our bio did not come back after the restart",
    );
  });
});
