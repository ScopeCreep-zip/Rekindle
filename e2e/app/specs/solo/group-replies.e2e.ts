// Plan G-replies: replies in a group chat of three (Ari in the app, Bea and
// Cara in the fleet). Bea writes; Ari replies to it from the composer (the
// bubble's Reply, the composer's quote, Send) and his reply shows Bea's
// message quoted under her name. Then Bea replies to a message nobody here
// holds (CLI --reply-to an unknown client id): Ari's window shows the reply
// with "Message unavailable", neither dropping the quote nor guessing it.
// Each state a shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, resolveIntent, shown, typeAndSend, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerAcceptGroup, peerReplyGroup, peerSendGroup, startFleet, stopFleet } from "../../fleet.ts";
import { createGroup, createIdentity, newUser, openGroup, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { befriendFleet } from "../../social.ts";

const GROUP = "Reply Room";
/** A client id no member holds: the parent of a reply to a message never seen. */
const UNSEEN = "019a0000-0000-7000-8000-00000000beef";

/** The quoted parent (or the unavailable note) on the bubble whose body includes `text`. */
async function quoteOn(text: string): Promise<string> {
  return (await browser.execute((t: string) => {
    const bubble = Array.from(document.querySelectorAll(".chat-message")).find((m) => (m.textContent ?? "").includes(t));
    return bubble?.querySelector(".reply-snippet")?.textContent?.trim() ?? "";
  }, text)) as string;
}

/**
 * The quote belongs to the reply: it is inside the message container holding
 * `text` and comes after that message's sender/time header (the quote was
 * drawn above the header, so it read as trailing the previous message).
 */
async function quoteOwned(text: string): Promise<string> {
  return (await browser.execute((t: string) => {
    const bubble = Array.from(document.querySelectorAll(".chat-message")).find((m) => (m.textContent ?? "").includes(t));
    if (!bubble) return "no message";
    const quote = bubble.querySelector(".reply-snippet");
    const header = bubble.querySelector(".chat-message-header");
    if (!quote) return "no quote inside the reply's message";
    if (!header) return "no header";
    return header.compareDocumentPosition(quote) & Node.DOCUMENT_POSITION_FOLLOWING ? "" : "the quote comes before the reply's header";
  }, text)) as string;
}

describe("replies in a group chat: quoted under the sender's name, or said unavailable", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("greply"), displayName: "Ari" };
  let peers: Peer[] = [];
  let groupId = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Bea", "Cara"]);
    await befriendFleet(browser, me, peers);
    await createGroup(browser, me, GROUP, ["Bea", "Cara"]);
    const ids = await Promise.all(peers.map((p) => peerAcceptGroup(p, GROUP)));
    groupId = ids[0];
    await openGroup(browser, GROUP);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("Ari replies to Bea from the composer, and his reply quotes her message under her name", async () => {
    const hers = `raid at nine? ${Date.now()}`;
    await peerSendGroup(peers[0], groupId, hers);
    const bubble = await byText(browser, ".chat-message", hers.slice(0, 30), NET_WAIT_MS);
    await bubble.moveTo();
    await press(browser, await bubble.$('.message-action-btn[aria-label="Reply"]'));
    await shown(browser, ".reply-preview", NET_WAIT_MS);
    const mine = `count me in ${Date.now()}`;
    await typeAndSend(browser, ".message-input-field", mine);
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
    await byText(browser, ".chat-message", mine.slice(0, 30), NET_WAIT_MS);
    let quote = "";
    await waitUntil(
      async () => {
        quote = await quoteOn(mine);
        return quote.startsWith("Bea") && quote.includes(hers.slice(0, 20));
      },
      NET_WAIT_MS,
      `Ari's reply never quoted Bea's message (quote: "${quote}")`,
    );
    expect(await quoteOwned(mine)).toBe("");
    await shot("reply-quoted");
  });

  it("a reply to a message nobody here holds shows 'Message unavailable'", async () => {
    const reply = `about that earlier one ${Date.now()}`;
    await peerReplyGroup(peers[0], groupId, reply, UNSEEN);
    await byText(browser, ".chat-message", reply.slice(0, 30), NET_WAIT_MS);
    let quote = "";
    await waitUntil(
      async () => {
        quote = await quoteOn(reply);
        return quote === "Message unavailable";
      },
      NET_WAIT_MS,
      `the reply to an unseen message never said it was unavailable (quote: "${quote}")`,
    );
    expect(quote).toBe("Message unavailable");
    expect(await quoteOwned(reply)).toBe("");
    await shot("reply-unavailable");
  });
});
