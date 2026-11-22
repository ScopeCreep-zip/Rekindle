// Plan E7.14 for E7.2 G3 (round 24): the Servers view is the Host's. A friend
// (a fleet peer) plays on a server: the view lists it under Friends' servers.
// Add to Favorites puts it under Favorites (the Host's view, refreshed by
// servers-event), where that row's Add to Favorites is then refused as already
// a favourite; Remove takes it out again. Refresh (G4) queries on the user's request
// only, and a server that never answers shows the Host's reason.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, resolveIntent, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, newUser, showSkinView, type User } from "../../journeys.ts";
import { befriendFleet } from "../../social.ts";

const SERVER = "192.0.2.10:27015";

/** The address's row in the Servers view section titled `section`, as text and its buttons' states. */
async function row(section: string): Promise<{ text: string; buttons: Record<string, boolean> } | null> {
  return (await browser.execute(
    (title: string, address: string) => {
      const group = document.querySelector(`.servers-view-section[aria-label="${title}"]`);
      const r = Array.from(group?.querySelectorAll<HTMLElement>(".servers-view-row") ?? []).find((e) =>
        (e.textContent ?? "").includes(address),
      );
      if (!r) return null;
      const buttons: Record<string, boolean> = {};
      for (const b of Array.from(r.querySelectorAll<HTMLElement>(".servers-view-action"))) {
        buttons[(b.textContent ?? "").trim()] = b.getAttribute("aria-disabled") !== "true";
      }
      return { text: (r.textContent ?? "").trim(), buttons };
    },
    section,
    SERVER,
  )) as { text: string; buttons: Record<string, boolean> } | null;
}

async function pressIn(section: string, label: string): Promise<void> {
  await browser.execute(
    (title: string, address: string, button: string) => {
      const group = document.querySelector(`.servers-view-section[aria-label="${title}"]`);
      const r = Array.from(group?.querySelectorAll<HTMLElement>(".servers-view-row") ?? []).find((e) =>
        (e.textContent ?? "").includes(address),
      );
      Array.from(r?.querySelectorAll<HTMLElement>(".servers-view-action") ?? [])
        .find((b) => (b.textContent ?? "").trim() === button)
        ?.click();
    },
    section,
    SERVER,
    label,
  );
}

describe("the Servers view (E7.2 G3)", function () {
  this.timeout(900_000);
  const me: User = newUser("servers");
  let peers: Peer[] = [];

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["ServerPal"]);
    await befriendFleet(browser, me, peers);
  });
  after(async () => {
    await stopFleet(peers);
  });

  it("lists a friend's server, adds it to Favorites, and removes it", async () => {
    await peers[0]!.cli(["game", "set", "Team Fortress 2", "--id", "4100", "--server", SERVER]);
    await showSkinView(browser, "servers");

    await waitUntil(async () => (await row("Friends' servers")) !== null, NET_WAIT_MS, "the friend's server is not listed");
    expect((await row("Friends' servers"))!.buttons["Add to Favorites"]).toBe(true);

    await pressIn("Friends' servers", "Add to Favorites");
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
    await waitUntil(async () => (await row("Favorites")) !== null, NET_WAIT_MS, "the server did not join Favorites");
    // Now a favourite: the Host refuses adding it again, and Remove is allowed.
    await waitUntil(
      async () => (await row("Friends' servers"))?.buttons["Add to Favorites"] === false,
      NET_WAIT_MS,
      "Add to Favorites was not refused for a favourite",
    );
    expect((await row("Favorites"))!.buttons.Remove).toBe(true);

    await pressIn("Favorites", "Remove");
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
    await waitUntil(async () => (await row("Favorites")) === null, NET_WAIT_MS, "the server stayed in Favorites");
  });

  // E7.2 G4: Refresh is the user's act (a query shows our IP to the server).
  // The fleet friend's server at a documentation address never answers, so
  // its row ends unanswered with the Host's reason, and Refresh is allowed
  // again once the run is over.
  it("refreshes on request and shows a server's lack of answer", async () => {
    await showSkinView(browser, "servers");
    await waitUntil(async () => (await row("Friends' servers")) !== null, NET_WAIT_MS, "the friend's server is not listed");
    const info = async (): Promise<{ kind: string; text: string } | null> =>
      (await browser.execute((address: string) => {
        const group = document.querySelector('.servers-view-section[aria-label="Friends\' servers"]');
        const r = Array.from(group?.querySelectorAll<HTMLElement>(".servers-view-row") ?? []).find((e) =>
          (e.textContent ?? "").includes(address),
        );
        const i = r?.querySelector<HTMLElement>(".servers-view-info");
        return i ? { kind: i.dataset.info ?? "", text: (i.textContent ?? "").trim() } : null;
      }, SERVER)) as { kind: string; text: string } | null;
    const refreshAllowed = async (): Promise<boolean> =>
      (await browser.execute(() => {
        const b = Array.from(document.querySelectorAll<HTMLElement>(".servers-view-toolbar .servers-view-action")).find(
          (e) => (e.textContent ?? "").trim() === "Refresh",
        );
        return !!b && b.getAttribute("aria-disabled") !== "true";
      })) as boolean;

    // Never queried until the user asks.
    expect(await info()).toBe(null);
    await waitUntil(refreshAllowed, NET_WAIT_MS, "Refresh was not allowed");
    await browser.execute(() =>
      Array.from(document.querySelectorAll<HTMLElement>(".servers-view-toolbar .servers-view-action"))
        .find((e) => (e.textContent ?? "").trim() === "Refresh")
        ?.click(),
    );
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
    await waitUntil(async () => (await info())?.kind === "unanswered", NET_WAIT_MS, "the server never reached unanswered");
    expect((await info())!.text.length).toBeGreaterThan(0);
    await waitUntil(refreshAllowed, NET_WAIT_MS, "Refresh stayed refused after the run");
  });
});
