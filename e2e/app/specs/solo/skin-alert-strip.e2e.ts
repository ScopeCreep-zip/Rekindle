// The skin alert on an open window (useSkin.ts showSkinError; plan SKC4c):
// a failed switch leaves the old skin and puts the Host's notice in a strip
// above the page, in flow, and the page scrolls beneath it instead of
// clipping (22-skin-frame.css). With the strip up, the last control of the
// buddy list (its bottom bar) and of a chat window (its Send) is still
// reachable: focused, it scrolls fully into view below the strip, and
// nothing covers it.
//
// The strip is placed here by the test, with the class and role the shell
// gives it: what is checked is the layout under it, not how a switch fails
// (scripts/tests/skin-reapply.mjs covers that).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY } from "../../../walk/nav.ts";
import { type Peer, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, newUser, openChat, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { befriendFleet } from "../../social.ts";

interface Reach {
  /** The last control in the page, by tag and accessible name. */
  control: string;
  /** Its box once focused, and the strip's foot and the viewport's height. */
  top: number;
  bottom: number;
  stripBottom: number;
  viewport: number;
  /** Whether the point at its centre is the control (or inside it). */
  onTop: boolean;
  /** Whether focus landed on it. */
  focused: boolean;
}

/** With a strip up in the window `browser` is in, focus its page's last control and measure it. */
async function lastControlUnderStrip(): Promise<Reach | string> {
  return (await browser.execute(async () => {
    const strip = document.createElement("div");
    strip.className = "skin-load-error";
    strip.setAttribute("role", "alert");
    strip.textContent = "skin alert strip (placed by the test)";
    document.body.prepend(strip);
    const root = document.getElementById("root");
    if (!root) return "no #root";
    const controls = Array.from(
      root.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      )
    ).filter((el) => el.getClientRects().length > 0);
    const last = controls[controls.length - 1];
    if (!last) return "no control in the page";
    last.focus();
    last.scrollIntoView({ block: "nearest" });
    await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
    const box = last.getBoundingClientRect();
    const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
    const name = last.getAttribute("aria-label") ?? last.getAttribute("title") ?? (last.textContent ?? "").trim().slice(0, 40);
    return {
      control: `${last.tagName.toLowerCase()} "${name}"`,
      top: box.top,
      bottom: box.bottom,
      stripBottom: strip.getBoundingClientRect().bottom,
      viewport: window.innerHeight,
      onTop: hit !== null && (hit === last || last.contains(hit)),
      focused: document.activeElement === last,
    };
  })) as Reach | string;
}

/** Take the strip down again. */
async function removeStrip(): Promise<void> {
  await browser.execute(() => {
    for (const strip of Array.from(document.querySelectorAll("body > .skin-load-error"))) strip.remove();
  });
}

function reachable(where: string, got: Reach | string): string[] {
  if (typeof got === "string") return [`${where}: ${got}`];
  const out: string[] = [];
  if (!got.focused) out.push(`${where}: ${got.control} did not take focus`);
  if (got.top < got.stripBottom - 1) out.push(`${where}: ${got.control} starts at ${got.top}, under the strip (foot ${got.stripBottom})`);
  if (got.bottom > got.viewport + 1) out.push(`${where}: ${got.control} ends at ${got.bottom}, past the window (${got.viewport})`);
  if (!got.onTop) out.push(`${where}: something covers ${got.control}`);
  return out;
}

describe("the skin alert strip covers no control (SKC4c)", function () {
  this.timeout(900_000);
  const me: User = { ...newUser("strip"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("with the strip up, the buddy list's and a chat window's last controls scroll into view below it", async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Bea"]);
    await befriendFleet(browser, me, peers);
    const faults: string[] = [];

    await switchTo(browser, BUDDY);
    await waitUntil(async () => (await browser.$(".app-frame .skin-tree")).isExisting(), NET_WAIT_MS, "the buddy list never laid its skin tree");
    const buddy = await lastControlUnderStrip();
    console.log(`[strip] buddy list: ${JSON.stringify(buddy)}`);
    await shot("buddy-list-strip");
    faults.push(...reachable("buddy list", buddy));
    await removeStrip();

    await openChat(browser, "Bea");
    const chat = await lastControlUnderStrip();
    console.log(`[strip] chat: ${JSON.stringify(chat)}`);
    await shot("chat-strip");
    faults.push(...reachable("chat", chat));
    await removeStrip();

    expect(faults).toEqual([]);
  });
});
