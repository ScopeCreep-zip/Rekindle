// Plan E7.14 for E3.4 S6c (round 24): a member's card change reaches the
// community's other members. Alice sets her name in her community (Settings →
// Overview, through the Host's review); Bob, a member, sees her by that name in
// the member list. Until then he sees her short fingerprint, never her identity
// name.

import { multiRemoteBrowser } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, resolveIntent, shown, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { createCommunity, createIdentity, createInvite, joinCommunity, newUser, openCommunity } from "../../journeys.ts";

const TOWN = "Name Town";
const NAME = "Alice of Name Town";

describe("a member's community name reaches the others (E3.4 S6c)", function () {
  this.timeout(900_000);
  const alice = newUser("alice");
  const bob = newUser("bob");

  it("shows Bob the name Alice set", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);

    await createCommunity(a, alice, TOWN);
    await openCommunity(a, TOWN);
    const link = await createInvite(a, alice, 1);
    await joinCommunity(b, bob, link, TOWN);
    await openCommunity(b, TOWN);

    // Before the change Bob never sees Alice's identity name.
    const names = async (): Promise<string[]> =>
      (await b.execute(() => Array.from(document.querySelectorAll(".member-name")).map((n) => (n.textContent ?? "").trim()))) as string[];
    await waitUntil(async () => (await names()).length >= 2, NET_WAIT_MS, "Bob's member list did not list both members");
    if ((await names()).includes(alice.displayName)) throw new Error("Bob saw Alice's identity name in the community");

    await press(a, await shown(a, '[title="Community Settings"]'));
    await press(a, await byText(a, ".modal-container .form-tab", "Overview"));
    await typeInto(await shown(a, ".my-name-section input"), NAME);
    await press(a, await shown(a, '.my-name-section button[type="submit"]'));
    await resolveIntent(a, { passphrase: alice.passphrase, appearMs: 5000 });

    await waitUntil(async () => (await names()).includes(NAME), NET_WAIT_MS, "Bob never saw the name Alice set");
  });
});
