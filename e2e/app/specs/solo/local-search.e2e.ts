// Plan E7.2 G11: the skin's Search opens a local search scoped to the
// window's view (the Host's OpenSearch), and a result opens its item. A fleet
// peer is a friend who sent a DM; on a pack derived from Symbiosis (skin-pack.ts
// searchPack: a Search on its Friends and Servers rows, which no built-in lays)
// Search on Friends finds the friend and opens their chat, Search on Servers
// and on Files (Symbiosis's own SearchFiles, Downloads' info) open the field
// scoped to that view; in the friend's chat window Mod+F (the same Search,
// through the Host) finds her message in that conversation. Each asserted
// state is a shot.

import { browser, expect } from "@wdio/globals";
import { MOD_KEY, NET_WAIT_MS, press, shown, switchTo, typeInto, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerSendDm, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, friendsView, importAndKeepSkin, newUser, showSkinView, skinShown, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { SEARCH_PACK, searchPack } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

/** Press the pack's node `id` in the buddy list, and the local search opens titled `title`. */
async function searchOpens(id: string, title: string): Promise<void> {
  await press(browser, await shown(browser, `[data-skin-node="${id}"]`, NET_WAIT_MS));
  await shown(browser, ".local-search-input", NET_WAIT_MS);
  expect((await (await browser.$(".modal-title")).getText()).trim()).toBe(title);
}

/** Close the open local search (Escape). */
async function closeSearch(): Promise<void> {
  await browser.keys("Escape");
  await waitUntil(async () => !(await (await browser.$(".local-search-input")).isExisting()), NET_WAIT_MS, "the local search did not close");
}

describe("the skin's Search opens a local search in the view's scope", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("localsearch"), displayName: "Ari" };
  let peers: Peer[] = [];

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Bea"]);
    const myKey = await befriendFleet(browser, me, peers);
    await peerSendDm(peers[0], myKey, "raid at nine");
    await openSettings(browser, "Appearance");
    const href = await importAndKeepSkin(browser, me, searchPack(), SEARCH_PACK.name);
    await switchTo(browser, BUDDY);
    await skinShown(browser, href);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("on Friends it finds the friend, and the result opens their chat", async () => {
    await friendsView(browser);
    await searchOpens(SEARCH_PACK.friends, "Search friends");
    await typeInto(await shown(browser, ".local-search-input"), "Bea");
    const hit = await shown(browser, ".local-search-hit", NET_WAIT_MS);
    await waitUntil(async () => (await hit.getText()).includes("Bea") && (await hit.getText()).includes("Friend"), NET_WAIT_MS, "no friend hit for Bea");
    await shot("friends-search");
    const before = await browser.getWindowHandles();
    await press(browser, hit);
    await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("chat-") && !before.includes(h), NET_WAIT_MS));
    await shown(browser, ".message-input-field", NET_WAIT_MS);
    await shot("friend-chat-opened");
  });

  it("in the friend's chat window Mod+F searches the conversation", async () => {
    // No pack lays a chat-window tree: Mod+F dispatches the skin's Search
    // through the Host, which scopes it to this conversation.
    await browser.keys([MOD_KEY, "f"]);
    await shown(browser, ".local-search-input", NET_WAIT_MS);
    expect((await (await browser.$(".modal-title")).getText()).trim()).toBe("Search this conversation");
    await typeInto(await shown(browser, ".local-search-input"), "raid");
    const message = await shown(browser, ".local-search-hit", NET_WAIT_MS);
    await waitUntil(async () => (await message.getText()).includes("raid at nine"), NET_WAIT_MS, "the conversation search did not find Bea's message");
    await shot("chat-search");
    await closeSearch();
    await browser.closeWindow();
    await switchTo(browser, BUDDY);
  });

  it("on Servers it opens the field scoped to servers", async () => {
    await showSkinView(browser, "servers");
    await searchOpens(SEARCH_PACK.servers, "Search servers");
    await shot("servers-search");
    await closeSearch();
  });

  it("on Files (Downloads' info) it opens the field scoped to files", async () => {
    await showSkinView(browser, "files");
    await searchOpens(SEARCH_PACK.files, "Search files");
    await shot("files-search");
    await closeSearch();
  });
});
