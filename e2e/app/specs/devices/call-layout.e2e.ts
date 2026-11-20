// Plan E7.14 S4: the call window (E7.12's "call" case) in the real app. The
// user befriends a real fleet peer, starts a voice call from the DM window
// (the peer, a headless daemon, never answers, so the call stays outgoing),
// pops the call out into its own window and checks it at each size, then
// hangs up. Needs the microphone (wdio.devices.conf.ts): if the OS refuses
// it, the call never starts and the spec fails naming what the app said.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, WAIT_MS, doubleClick, press, shown, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerRequestFriend, startFleet, stopFleet } from "../../fleet.ts";
import { acceptRequest, createIdentity, friendCode, friendRow, newUser, pendingRequest } from "../../journeys.ts";
import { CALL, checkWindow } from "../../windows.ts";

describe("call window", () => {
  const me = newUser("caller");
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("an outgoing voice call, popped out, at each size", async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["callee"]);
    const hello = "call me";
    await peerRequestFriend(peers[0], await friendCode(browser), hello);
    await pendingRequest(browser, hello);
    await acceptRequest(browser, me, hello);

    const before = await browser.getWindowHandles();
    await doubleClick(browser, await friendRow(browser, peers[0].displayName));
    await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("chat-") && !before.includes(h)));
    await press(browser, await shown(browser, `[aria-label="Start voice call with ${peers[0].displayName}"]`, NET_WAIT_MS));
    try {
      await shown(browser, ".active-call-panel-header", WAIT_MS * 3);
    } catch {
      // A refused microphone (macOS TCC for an app a Claude session
      // launched) or any other start failure: name what the window shows.
      const said = await browser.execute(() =>
        Array.from(document.querySelectorAll("[role=alert], [role=status], .toast"))
          .map((el) => el.textContent?.trim() ?? "")
          .filter(Boolean)
          .join(" | "),
      );
      throw new Error(`the call never started; the window says: ${said || "nothing"}`);
    }

    const open = await browser.getWindowHandles();
    await press(browser, await shown(browser, '[aria-label="Pop out call"]'));
    await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("call-") && !open.includes(h)));
    await shown(browser, ".active-call-controls");
    await expect(await checkWindow(browser, CALL, { withData: true })).toEqual({});

    await press(browser, await shown(browser, '[aria-label="Hang up"]'));
    await waitUntil(async () => !(await browser.getWindowHandles()).some((h) => h.startsWith("call-")), NET_WAIT_MS, "the call window stayed open after Hang up");
  }).timeout(900_000);
});
