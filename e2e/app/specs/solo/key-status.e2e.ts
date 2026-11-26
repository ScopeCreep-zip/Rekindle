// FX3 (no local key mint): a member missing a rotated community key waits
// for a key holder, and is told. Cara (fleet) creates a community; Ari (the
// app), Bea, Dan and Eve (fleet) join. Ari logs out; Cara kicks Bea, and
// the member the cascade elects rotates the key at once (rekindle-mek-
// rotation rotate.rs on a departure; ADR 0018's majority ballots are E3.5,
// and five members keep the spec valid then: a majority without Ari). Once
// Cara's `community info` reports generation 2, Cara, Dan and Eve, every
// holder whoever rotated, go offline; Ari unlocks and learns generation 2 from the governance state,
// with no holder online: his community window says "Waiting for the
// community key from a member…" (Pending; shot). Cara comes back, answers his
// key request, and the notice goes (Current; shot). The Host path is netsim
// key_status_after_unlock's.

import { browser } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerJoinCommunity, peerKick, peerMemberCount, peerOwnPseudonym, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, logOut, newUser, openCommunity, selectChannel, unlock, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Key Watch ${Date.now() % 100000}`;

/** The open community window's key notice text, or "" when none is drawn. */
async function keyNotice(): Promise<string> {
  return (await browser.execute(() => {
    const n = document.querySelector<HTMLElement>(".key-pending-notice");
    return n && n.getBoundingClientRect().height > 0 ? (n.textContent ?? "").trim() : "";
  })) as string;
}

describe("a rotated key we lack: pending until a holder answers, then current", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("keystatus"), displayName: "Ari" };
  let peers: Peer[] = [];
  let anchor = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara", "Bea", "Dan", "Eve"]);
    const [cara, ...others] = peers;
    anchor = await peerCreateCommunity(cara, COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(cara, anchor, 1), COMMUNITY);
    for (const peer of others) await peerJoinCommunity(peer, await peerInvite(cara, anchor, 1));
    await waitUntil(async () => (await peerMemberCount(cara, anchor)) >= 5, NET_WAIT_MS * 3, "Cara never counted five members");
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("while Ari is away Cara kicks Bea, generation 2 is minted without Ari, and every key holder goes offline", async () => {
    const [cara, bea, dan, eve] = peers;
    await logOut(browser, me);
    // The CLI kicks by pseudonym (the 64 hex community info serves).
    await peerKick(cara, anchor, await peerOwnPseudonym(bea, anchor));
    // The key holders go offline only once Cara holds generation 2 (her
    // `community info` `mekGeneration`, the window's source) and her kick and
    // the rotation have landed on the DHT: no op of hers still publishing
    // (`opPublications` empty) and no barrier holding her writes (`catchUp`
    // absent). Her member count flips locally before publication.
    let last = "";
    await waitUntil(
      async () => {
        const info = ((await cara.cli(["community", "info", "--community", anchor])) ?? {}) as Record<string, unknown>;
        const members = (info.memberCount ?? info.member_count) as number | undefined;
        const publishing = (info.opPublications ?? info.op_publications ?? {}) as Record<string, unknown>;
        const catchUp = info.catchUp ?? info.catch_up;
        const generation = (info.mekGeneration ?? info.mek_generation) as number | undefined;
        last = JSON.stringify({ members, generation, publishing, catchUp });
        return members === 4 && generation === 2 && Object.keys(publishing).length === 0 && catchUp == null;
      },
      NET_WAIT_MS * 2,
      "",
    ).catch(() => {
      throw new Error(`generation 2 never reached Cara and landed (Cara's community info: ${last})`);
    });
    console.log(`[key] Cara at generation 2, ops landed: ${last}`);
    // Every member that holds generation 2 goes offline: no holder can answer Ari.
    await Promise.all([cara.pause(), dan.pause(), eve.pause()]);
  });

  it("Ari unlocks with no key holder online: the window says it waits for the key", async () => {
    await unlock(browser, me);
    await openCommunity(browser, COMMUNITY);
    await selectChannel(browser, "general");
    let notice = "";
    await waitUntil(async () => (notice = await keyNotice()).length > 0, NET_WAIT_MS * 2, "").catch(() => {
      throw new Error("no key-pending notice after unlock with no holder online");
    });
    console.log(`[key] pending: "${notice}"`);
    if (!notice.startsWith("Waiting for the community key from a member")) throw new Error(`unexpected notice: ${notice}`);
    await shot("key-pending");
  });

  it("Cara comes back and answers: the key is current and the notice goes", async () => {
    // Lead (the ~38 s unanswered executeScript in this step, twice): a page
    // heartbeat (requestAnimationFrame and a 250 ms timer) records every gap
    // over 500 ms with the window's visibility and focus, and a
    // MutationObserver counts the window's DOM changes, so a throttled
    // background page (a gap, no changes) tells itself from a re-render
    // flood (changes spike). Each notice poll is timed too.
    await browser.execute(() => {
      const w = window as unknown as { __hb?: { gaps: string[]; mutations: number; last: number } };
      if (w.__hb) return;
      const hb = { gaps: [] as string[], mutations: 0, last: performance.now() };
      w.__hb = hb;
      const beat = (src: string) => {
        const now = performance.now();
        if (now - hb.last > 500) hb.gaps.push(`${src} gap ${Math.round(now - hb.last)} ms ending ${new Date().toISOString()} visibility=${document.visibilityState} focus=${document.hasFocus()} mutations=${hb.mutations}`);
        hb.last = now;
      };
      const frame = () => {
        beat("raf");
        requestAnimationFrame(frame);
      };
      requestAnimationFrame(frame);
      setInterval(() => beat("timer"), 250);
      new MutationObserver((records) => {
        hb.mutations += records.length;
      }).observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
    });
    const handle = await browser.getWindowHandle();
    console.log(`[key-hb] polling window ${handle}`);
    await peers[0].resume();
    const polls: string[] = [];
    await waitUntil(
      async () => {
        const t0 = Date.now();
        const notice = await keyNotice();
        const ms = Date.now() - t0;
        if (ms > 1000) polls.push(`poll took ${ms} ms ending ${new Date().toISOString()}`);
        return notice === "";
      },
      NET_WAIT_MS * 3,
      "the key never became current after Cara came back",
    );
    const hb = (await browser.execute(() => (window as unknown as { __hb?: { gaps: string[]; mutations: number } }).__hb ?? null)) as { gaps: string[]; mutations: number } | null;
    console.log(`[key-hb] slow polls: ${JSON.stringify(polls)}`);
    console.log(`[key-hb] page gaps: ${JSON.stringify(hb?.gaps ?? [])}; mutations in all: ${hb?.mutations ?? "none"}`);
    console.log("[key] current");
    await shot("key-current");
  });
});
