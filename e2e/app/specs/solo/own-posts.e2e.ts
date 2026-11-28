// Plan CP1 (round 24): alone in a new community, every post of ours stays
// on screen once. The window inserts our post at the send, a history read
// that began before the post was stored must not drop it, and the Host's
// echo (`ChannelMessageEvent::New`) reconciles it by message id rather than
// adding a second row. Linux's r23 run lost the second of three posts
// (skins.e2e.ts:129): a replacing read dropped it and the echo was ignored.

import { browser, expect } from "@wdio/globals";
import { sleep } from "../../../walk/helpers.ts";
import { createCommunity, createIdentity, createTextChannel, newUser, openCommunity, postMessage, selectChannel } from "../../journeys.ts";

const POSTS = ["First post, nobody here yet.", "Second post, still alone.", "Third post, for the record."];

/** How many of the channel's rows show `text`. */
async function rowsWith(text: string): Promise<number> {
  return (await browser.execute(
    (t: string) =>
      Array.from(document.querySelectorAll(".community-main .chat-message")).filter((m) => (m.textContent ?? "").includes(t)).length,
    text,
  )) as number;
}

describe("our own posts in a community we are alone in", function () {
  this.timeout(600_000);
  const me = newUser("ownposts");

  it("shows each of three posts exactly once, and keeps them", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, "Own Posts Town");
    await openCommunity(browser, "Own Posts Town");
    await createTextChannel(browser, me, "solo");
    await selectChannel(browser, "solo");
    // postMessage waits for each post to render before the next.
    for (const post of POSTS) await postMessage(browser, me, post);
    // The Host's echoes and the channel's history reads land after the
    // sends; a dropped or doubled row shows once they have.
    await sleep(8_000);
    for (const post of POSTS) expect(await rowsWith(post)).toBe(1);
  });
});
