// Contract VN2: a group call's members are named as the Host serves them
// (participants: { publicKey, displayName }), never by a slice of a key.
// Three real apps (wdio.devices-peers.conf.ts: alice, bob, carol), since a
// daemon places no call before E4.1. Bob, friends with both, starts a group
// voice call to Alice and Carol through the Host (`start_group_call`, the
// command the Start Group Call panel sends; the call's own UI is what is
// tested here). Alice's incoming banner names Bob and sits inside her narrow
// buddy list without covering its chrome; she accepts, and her call grid
// names Bob, Carol by her spaced fingerprint (not Alice's friend, and a call
// with no group DM has no roster, so the Host's name order ends at the
// fingerprint; passing Bob's alias for her would leak it, addendum 2) and
// herself with "(you)" (the Host's isSelf), with no run of 8+ hex
// anywhere in the call's UI, and nothing in the window shows as a sliver
// (a text or status element visible at under 4 px tall). Accepting a call
// opens the microphone: Linux only (no TCC prompt on the Mac).

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { byText, NET_WAIT_MS, press, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY } from "../../../walk/nav.ts";
import {
  acceptRequest,
  createIdentity,
  friendCode,
  friendRow,
  newUser,
  pendingRequest,
  requestFriend,
  type User,
} from "../../journeys.ts";
import { painted } from "../../painted.ts";
import { shot } from "../../shots.ts";

const app = (name: "alice" | "bob" | "carol"): Browser =>
  multiRemoteBrowser.getInstance(name) as unknown as Browser;

/** `b`'s identity as the Host serves it: its key, and the spaced fingerprint
 *  the Host names it by where no friend or roster name applies. */
async function identity(b: Browser): Promise<{ publicKey: string; fingerprint: string }> {
  return (await b.execute(async () => {
    const { invoke } = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(cmd: string): Promise<{ value: { publicKey: string; fingerprint: string } }>;
        };
      }
    ).__TAURI_INTERNALS__;
    const { publicKey, fingerprint } = (await invoke("get_identity")).value;
    return { publicKey, fingerprint };
  })) as { publicKey: string; fingerprint: string };
}

/** `from` and `to` become friends: `from` asks with `to`'s code, `to` accepts. */
async function befriend(from: Browser, fromUser: User, to: Browser, toUser: User): Promise<void> {
  const hello = `${fromUser.displayName} to ${toUser.displayName} ${Date.now()}`;
  await requestFriend(from, fromUser, await friendCode(to), hello);
  await pendingRequest(to, hello);
  await acceptRequest(to, toUser, hello);
  await friendRow(from, toUser.displayName);
}

describe("a group call names its members as the Host serves them (VN2)", function () {
  this.timeout(1_800_000);
  const alice: User = { ...newUser("gc-alice"), displayName: "Alice" };
  const bob: User = { ...newUser("gc-bob"), displayName: "Bob" };
  const carol: User = { ...newUser("gc-carol"), displayName: "Carol" };

  it("Alice's incoming banner and call grid name Bob, Carol's fingerprint and herself, never by a key slice", async () => {
    const a = app("alice");
    const b = app("bob");
    const c = app("carol");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob), createIdentity(c, carol)]);
    await befriend(b, bob, a, alice);
    await befriend(b, bob, c, carol);
    const [{ publicKey: aliceKey }, { publicKey: carolKey, fingerprint: carolName }] =
      await Promise.all([identity(a), identity(c)]);

    // Bob starts the group voice call to Alice and Carol through the Host.
    await switchTo(b, BUDDY);
    await b.execute(
      async (keys: string[]) => {
        const { invoke } = (
          window as unknown as {
            __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> };
          }
        ).__TAURI_INTERNALS__;
        await invoke("start_group_call", { participantPubkeys: keys, video: false, groupId: null });
      },
      [aliceKey, carolKey]
    );

    // Alice's incoming banner names the caller as the Host serves him.
    await switchTo(a, BUDDY);
    const banner = await shown(a, ".group-call-incoming-banner", NET_WAIT_MS);
    const bannerText = (await banner.getText()).replace(/\s+/g, " ");
    console.log(`[group-call] Alice's banner: ${bannerText}`);
    const place = (await a.execute(() => {
      const box = document.querySelector(".group-call-incoming-banner")!.getBoundingClientRect();
      const chrome = Array.from(
        document.querySelectorAll(
          '.titlebar, .menu-bar, [data-skin-action="minimize"], [data-skin-action="maximize"], [data-skin-action="close"], [data-skin-action^="menu:"]'
        )
      ).filter((e) => (e as HTMLElement).offsetParent !== null);
      const chromeBottom = Math.max(0, ...chrome.map((e) => e.getBoundingClientRect().bottom));
      const meta = document.querySelector(".group-call-banner-meta")?.getBoundingClientRect();
      return {
        inWindow:
          box.left >= 0 &&
          box.top >= 0 &&
          box.right <= window.innerWidth &&
          box.bottom <= window.innerHeight,
        belowChrome: box.top >= chromeBottom,
        metaClear: meta ? box.bottom - meta.bottom >= 8 : false,
        box: [box.left, box.top, box.right, box.bottom].map(Math.round),
        chromeBottom: Math.round(chromeBottom),
        window: [window.innerWidth, window.innerHeight],
      };
    })) as {
      inWindow: boolean;
      belowChrome: boolean;
      metaClear: boolean;
      box: number[];
      chromeBottom: number;
      window: number[];
    };
    console.log(`[group-call] banner ${JSON.stringify(place)}`);
    // Seen, not only laid out: hiding the banner changes most of its box's pixels.
    const bannerPaint = await painted(a, ".group-call-incoming-banner");
    console.log(`[group-call] banner painted: ${JSON.stringify(bannerPaint)}`);
    await shot("group-call-incoming");

    // She accepts: her call grid names each member. Every sample counts, from
    // the first tile drawn: each tile's name is one the Host served (Bob,
    // Carol's fingerprint, herself with "(you)"), never empty or anything else.
    const served = new Set(["Bob", carolName, "Alice (you)"]);
    const offSamples: string[][] = [];
    let samples = 0;
    const sample = async (): Promise<string[]> => {
      const now = (await a.execute(() =>
        Array.from(document.querySelectorAll(".group-call-tile-name")).map((n) =>
          (n.textContent ?? "").replace(/\s+/g, " ").trim()
        )
      )) as string[];
      samples += 1;
      if (now.some((n) => !served.has(n)) && offSamples.length < 10) offSamples.push(now);
      return now;
    };
    await press(a, await byText(a, ".group-call-incoming-banner button", "Accept"));
    let names: string[] = [];
    await waitUntil(
      async () => {
        names = await sample();
        return names.includes("Bob") && names.includes(carolName) && names.includes("Alice (you)");
      },
      NET_WAIT_MS,
      ""
    ).catch(() => {
      throw new Error(
        `Alice's call grid never named Bob, Carol (${carolName}) and Alice (you) (it shows ${JSON.stringify(names)})`
      );
    });
    // And it stays so: 5 s more of samples.
    for (const until = Date.now() + 5_000; Date.now() < until; ) {
      names = await sample();
      await new Promise((r) => setTimeout(r, 250));
    }
    console.log(
      `[group-call] ${samples} grid samples; off samples (a tile name empty or not served): ${JSON.stringify(offSamples)}`
    );
    const callText = (await a.execute(() =>
      Array.from(
        document.querySelectorAll(
          ".group-call-incoming-banner, .group-call-grid, .active-call-panel"
        )
      )
        .map((el) => el.textContent ?? "")
        .join(" ")
    )) as string;
    const hexRuns = callText.toLowerCase().match(/[0-9a-f]{8,}/g) ?? [];
    console.log(
      `[group-call] Alice's grid: ${JSON.stringify(names)}; hex runs in the call's UI: ${hexRuns.length}`
    );
    const slivers = (await a.execute(() => {
      const visible = (el: Element): DOMRect | null => {
        let box = el.getBoundingClientRect();
        if (box.width === 0 || box.height === 0) return null;
        for (let p = el.parentElement; p; p = p.parentElement) {
          const st = getComputedStyle(p);
          if (st.overflowX === "visible" && st.overflowY === "visible") continue;
          const r = p.getBoundingClientRect();
          const top = Math.max(box.top, r.top);
          const bottom = Math.min(box.bottom, r.bottom);
          const left = Math.max(box.left, r.left);
          const right = Math.min(box.right, r.right);
          if (bottom <= top || right <= left) return null;
          box = new DOMRect(left, top, right - left, bottom - top);
        }
        return box;
      };
      const shows = (el: Element): boolean =>
        /status-dot|dot/.test(typeof el.className === "string" ? el.className : "") ||
        Array.from(el.childNodes).some(
          (n) => n.nodeType === Node.TEXT_NODE && (n.textContent ?? "").trim() !== ""
        );
      const chain = (el: Element): string[] => {
        const out: string[] = [];
        for (let p: Element | null = el; p && out.length < 6; p = p.parentElement) {
          const r = p.getBoundingClientRect();
          const cls =
            typeof p.className === "string" ? p.className.split(/\s+/).slice(0, 2).join(".") : "";
          out.push(
            `${p.tagName.toLowerCase()}${cls ? `.${cls}` : ""}[${Math.round(r.top)}-${Math.round(r.bottom)}]`
          );
        }
        return out;
      };
      const found: { el: string[]; visible: number[] }[] = [];
      for (const el of Array.from(document.body.querySelectorAll("*"))) {
        if (getComputedStyle(el).visibility === "hidden" || !shows(el)) continue;
        const v = visible(el);
        if (v && v.height > 0 && v.height < 4)
          found.push({
            el: chain(el),
            visible: [v.left, v.top, v.right, v.bottom].map(Math.round),
          });
      }
      return found;
    })) as { el: string[]; visible: number[] }[];
    console.log(`[group-call] slivers (visible under 4 px tall): ${JSON.stringify(slivers)}`);
    await shot("group-call-grid");

    expect({
      caller: bannerText.includes("Bob"),
      names: names.includes("Bob") && names.includes(carolName) && names.includes("Alice (you)"),
      offSamples,
      hexRuns,
      bannerInWindow: place.inWindow,
      bannerBelowChrome: place.belowChrome,
      bannerTextClear: place.metaClear,
      bannerPainted: bannerPaint.changed >= bannerPaint.total / 2,
      slivers,
    }).toEqual({
      caller: true,
      names: true,
      offSamples: [],
      hexRuns: [],
      bannerInWindow: true,
      bannerBelowChrome: true,
      bannerTextClear: true,
      bannerPainted: true,
      slivers: [],
    });
  });
});
