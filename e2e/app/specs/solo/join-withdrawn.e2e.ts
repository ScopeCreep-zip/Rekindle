// Plan JW (E3.1/JN1): a join the inviter refuses after it returned. Cara
// (fleet) creates a community and one single-use invite, then goes offline.
// Bea (fleet) and Ari (the app) both join by it: neither exchange is
// answered, so each join stands from the DHT and returns, its exchange
// queued. Ari logs out, so only Bea's retry can reach Cara; Cara comes back
// and admits Bea, which takes the one use. Ari unlocks: his retried exchange
// is the second redemption, Cara refuses it, and the Host withdraws the join
// (netsim join_withdrawn.rs, with real peers: the inviter offline in place
// of Fault::Unanswered, the logout in place of the record-write fault).
// Ari's buddy list must drop the community and say the Host's words, once.
// Each state a shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerJoinCommunity, peerMemberCount, startFleet, stopFleet } from "../../fleet.ts";
import { communitiesTab, createIdentity, joinCommunity, logOut, newUser, unlock, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Refusal Hall ${Date.now() % 100000}`;
/** The Host's words for an invite whose one use is taken (rekindle_types JoinRefusal::InviteUsedUp withdrawn_text). */
const USED_UP = "This invite has already been used";

/** From now on, record in the page the text of every toast added (a toast dismisses itself, so it is counted as it appears). */
async function recordToasts(): Promise<void> {
  await browser.execute(() => {
    const w = window as unknown as { __toasts?: string[] };
    w.__toasts = [];
    const seen = new WeakSet<Element>();
    const note = (): void => {
      for (const t of Array.from(document.querySelectorAll(".toast-item"))) {
        if (seen.has(t)) continue;
        seen.add(t);
        w.__toasts!.push((t.textContent ?? "").trim());
      }
    };
    note();
    new MutationObserver(note).observe(document.body, { subtree: true, childList: true });
  });
}

/** Every toast added since `recordToasts`, by text. */
async function toasts(): Promise<string[]> {
  return (await browser.execute(() => (window as unknown as { __toasts?: string[] }).__toasts ?? [])) as string[];
}

describe("a join the inviter refuses after it returned is withdrawn, in the Host's words", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("jw"), displayName: "Ari" };
  let peers: Peer[] = [];
  let anchor = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara", "Bea"]);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("both joins return while the inviter is offline", async () => {
    const [cara, bea] = peers;
    anchor = await peerCreateCommunity(cara, COMMUNITY);
    const link = await peerInvite(cara, anchor, 1);
    await cara.pause();
    await peerJoinCommunity(bea, link);
    await joinCommunity(browser, me, link, COMMUNITY);
    await shot("joined-while-inviter-offline");
  });

  it("the inviter returns and admits Bea while Ari is away", async () => {
    const [cara] = peers;
    await logOut(browser, me);
    await cara.resume();
    await waitUntil(async () => (await peerMemberCount(cara, anchor)) >= 2, NET_WAIT_MS * 2, "Cara never admitted Bea");
  });

  it("Ari unlocks: his retried join is refused, the community leaves his list and the Host's words show once", async () => {
    await unlock(browser, me);
    await recordToasts();
    await communitiesTab(browser);
    let seen: string[] = [];
    await waitUntil(
      async () => {
        seen = await toasts();
        return seen.some((t) => t.includes(USED_UP));
      },
      NET_WAIT_MS * 2,
      "",
    ).catch(() => {
      throw new Error(`no withdrawal notice (toasts: ${JSON.stringify(seen)})`);
    });
    expect(seen.filter((t) => t.includes(USED_UP))).toHaveLength(1);
    await waitUntil(
      async () =>
        !((await browser.execute(
          (n: string) => Array.from(document.querySelectorAll(".community-item")).some((r) => (r.textContent ?? "").includes(n)),
          COMMUNITY.slice(0, 24),
        )) as boolean),
      NET_WAIT_MS,
      `${COMMUNITY} stayed in Ari's list`,
    );
    await shot("withdrawn-notice");
    // Never a second notice for the same withdrawal.
    await browser.pause(5000);
    expect((await toasts()).filter((t) => t.includes(USED_UP))).toHaveLength(1);
  });
});
