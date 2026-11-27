// Plan E7.14 for E3.4 S9 (round 24): channel typing is the Host's start and
// stop, with no frontend timer. Bob types in a channel of Alice's community
// (not sending): Alice sees him typing. Bob sends: the Host stops his typing
// with the message, and Alice's indicator clears within 3.5 s of the send.
// The deleted window timer held an indicator 5 s after the last start (here,
// the send), so a clear that fast is the Host's stop, not a timer.

import { multiRemoteBrowser } from "@wdio/globals";
import { CHANNEL_COMPOSER, NET_WAIT_MS, present, resolveIntent, shown, typeAndSend, typeInto, waitUntil } from "../../../walk/helpers.ts";
import {
  createCommunity,
  createIdentity,
  createInvite,
  createTextChannel,
  joinCommunity,
  newUser,
  openCommunity,
  selectChannel,
} from "../../journeys.ts";

const TOWN = "Typing Town";
const CHANNEL = "chatter";

describe("channel typing from the Host (E3.4 S9)", function () {
  this.timeout(900_000);
  const alice = newUser("alice");
  const bob = newUser("bob");

  it("shows Bob typing to Alice, and clears it when his message goes out", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);

    await createCommunity(a, alice, TOWN);
    await openCommunity(a, TOWN);
    await createTextChannel(a, alice, CHANNEL);
    await selectChannel(a, CHANNEL);
    const link = await createInvite(a, alice, 1);

    await joinCommunity(b, bob, link, TOWN);
    await openCommunity(b, TOWN);
    await selectChannel(b, CHANNEL);

    // Bob types without sending: Alice sees someone typing.
    await typeInto(await shown(b, CHANNEL_COMPOSER), "on my way");
    await shown(a, ".community-main .typing-indicator", NET_WAIT_MS);

    // Bob sends straight after typing (his last typing start is at the
    // send): the Host ends his typing with the message. The old window
    // timer would have held the indicator about 5 s past this point.
    await typeAndSend(b, CHANNEL_COMPOSER, "on my way, two minutes");
    const sent = Date.now();
    await resolveIntent(b, { passphrase: bob.passphrase, appearMs: 1500 });
    // Within 3.5 s of the send itself, wherever the review wait above ended.
    await waitUntil(
      async () => !(await present(a, ".community-main .typing-indicator")),
      Math.max(500, 3_500 - (Date.now() - sent)),
      "Alice still saw Bob typing 3.5 s after his message",
    );
    // Alice has the message itself.
    await waitUntil(
      async () =>
        ((await a.execute(() => Array.from(document.querySelectorAll(".community-main .chat-message")).map((m) => m.textContent ?? ""))) as string[]).some(
          (t) => t.includes("two minutes"),
        ),
      NET_WAIT_MS,
      "Alice did not receive Bob's message",
    );
    console.log(`[typing] cleared within ${Date.now() - sent} ms of the send`);
  });
});
