// Group voice split (pack-grounded: Symbiosis GroupChatWindow.xml:112-165):
// in a group window, the pack's voice tab reads Call with no call, Join while
// the group's call rings, and Leave once joined: one art node per state
// (ChatButtonCall / ChatButtonAnswerCall / ChatButtonLeaveCall). Alice and Bob (two apps,
// multiremote) and Cara (fleet) are a group of three, both apps under
// Symbiosis. Bob presses his group window's Call tab (HostGroupVoiceChat,
// which opens StartGroupCall) and ticks exactly the group's other members,
// Alice and Cara: a call whose participants match the group's roster is the
// group's call (wt/gv Inputs::group_call). Alice's group window then shows
// Join (the joinGroup op); she presses it and it shows Leave. Each state a
// shot. Then (GV3) Leave: Alice leaves, Bob leaves and the call ends; Bob
// rings again and cancels his own ring. Starting and joining a call opens the
// microphone: Linux only (no TCC prompt may be raised on the Mac).

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, press, resolveIntent, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerAcceptGroup, startFleet, stopFleet } from "../../fleet.ts";
import { acceptRequest, createGroup, createIdentity, friendCode, friendRow, newUser, openGroup, pendingRequest, requestFriend, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const GROUP = `Voice Squad ${Date.now() % 100000}`;
/** A second group with the same members (GV2: a call is the group's by its id, never by who is in it). */
const TWIN = `Twin Squad ${Date.now() % 100000}`;

interface Tab {
  id: string;
  role: string;
  action: string;
  /** Drawn disabled (`aria-disabled`): a press does nothing, the Host's reason is its tooltip. */
  disabled: boolean;
  reason: string;
}

/**
 * The pack's voice control for each state (Symbiosis GroupChatWindow, r28
 * group-tree diagnostic): art buttons with the word baked in (no label), one
 * node per state, the others hidden. Found by node id, never by text.
 */
const VOICE_NODE = {
  Call: "ChatButtonCall",
  Join: "ChatButtonAnswerCall",
  Leave: "ChatButtonLeaveCall",
  // Our own ring still going out (GV3, action cancelVoiceChat).
  Cancel: "ChatButtonCancelCall",
} as const;
type VoiceState = keyof typeof VOICE_NODE;

/** The voice control `state`'s node in `b`'s window when it is drawn (not hidden, with a box), else null. */
async function voiceNode(b: Browser, state: VoiceState): Promise<Tab | null> {
  return (await b.execute((id: string) => {
    const n = document.querySelector<HTMLElement>(`[data-skin-node="${id}"]`);
    if (!n || n.closest("[hidden]") !== null) return null;
    const r = n.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return null;
    return {
      id,
      role: n.dataset.skinRole ?? "",
      action: n.dataset.skinAction ?? "",
      disabled: n.getAttribute("aria-disabled") === "true",
      reason: n.getAttribute("title") ?? n.getAttribute("aria-description") ?? "",
    };
  }, VOICE_NODE[state])) as Tab | null;
}

/** Wait until `b`'s window draws the voice control for `state`; logs it as `[gv]`. */
async function voiceTab(b: Browser, who: string, state: VoiceState): Promise<Tab> {
  let tab: Tab | null = null;
  await waitUntil(async () => (tab = await voiceNode(b, state)) !== null, NET_WAIT_MS, `${who}'s group window never drew ${state} (${VOICE_NODE[state]})`);
  console.log(`[gv] ${who} ${state}: ${JSON.stringify(tab)}`);
  return tab!;
}

/** The voice control state `b`'s window draws now, if any. */
async function voiceState(b: Browser): Promise<VoiceState | null> {
  for (const state of Object.keys(VOICE_NODE) as VoiceState[]) {
    if ((await voiceNode(b, state)) !== null) return state;
  }
  return null;
}

/** Wait until `b`'s window draws one of `states`; logs it as `[gv]`. */
async function voiceTabOneOf(b: Browser, who: string, states: VoiceState[]): Promise<VoiceState> {
  let now: VoiceState | null = null;
  await waitUntil(
    async () => (now = await voiceState(b)) !== null && states.includes(now),
    NET_WAIT_MS,
    `${who}'s group window never drew ${states.join(" or ")} (last: ${now ?? "none"})`,
  );
  console.log(`[gv] ${who}: ${now}`);
  return now!;
}

/**
 * Press the Start Group Call panel's Call in `b`'s window and record what
 * happens (lead, gv6: the panel stayed open and nothing logged): the button
 * before, then, for up to 8 s, whether the panel is open, the button's label
 * ("Calling…" while start_group_call is in flight) and every toast. Logged
 * as `[gv]`; throws with that record when the panel never closes.
 */
async function pressPanelCall(b: Browser, who: string): Promise<void> {
  const read = async () =>
    (await b.execute(() => {
      const btn = document.querySelector<HTMLButtonElement>(".modal-container .modal-footer button.form-btn-primary");
      return {
        open: !!document.querySelector(".modal-container"),
        button: btn ? { text: (btn.textContent ?? "").trim(), disabled: btn.disabled } : null,
        toasts: Array.from(document.querySelectorAll(".toast-item")).map((t) => (t.textContent ?? "").trim()),
      };
    })) as { open: boolean; button: { text: string; disabled: boolean } | null; toasts: string[] };
  const before = await read();
  console.log(`[gv] ${who} panel Call before: ${JSON.stringify(before)}`);
  if (!before.button) throw new Error(`${who}: the Start Group Call panel has no Call button`);
  if (before.button.disabled) throw new Error(`${who}: the panel's Call is disabled (${JSON.stringify(before)})`);
  await press(b, await b.$(".modal-container .modal-footer button.form-btn-primary"));
  const seen: string[] = [];
  let last = before;
  for (let i = 0; i < 16; i++) {
    await b.pause(500);
    last = await read();
    const line = JSON.stringify(last);
    if (seen[seen.length - 1] !== line) seen.push(line);
    if (!last.open) break;
  }
  console.log(`[gv] ${who} panel Call after: ${seen.join(" → ")}`);
  if (last.open) throw new Error(`${who}: the panel stayed open after Call (${seen.join(" → ")})`);
}

/** Press tree node `id` in `b`'s window. */
async function pressNode(b: Browser, id: string): Promise<void> {
  await press(b, await b.$(`[data-skin-node="${id}"]`));
}

async function symbiosis(b: Browser, user: User): Promise<string> {
  await openSettings(b, "Appearance");
  const pack = builtinPacks().find((p) => p.file === "symbiosis");
  if (!pack) throw new Error("no built-in pack symbiosis");
  return applySkin(b, user, pack.name);
}

/**
 * Record, in `b`'s window from now on, every IPC call through
 * `window.__TAURI_INTERNALS__.invoke` (its answer or refusal), every console
 * warning and error, and every unhandled error or rejection (lead, gv10:
 * proven against a press that works before it is trusted on Cancel).
 */
async function recordIpc(b: Browser): Promise<void> {
  await b.execute(`if (!window.__gvCalls) {
  window.__gvCalls = [];
  const text = (a) => (a instanceof Error ? a.message : typeof a === "string" ? a : JSON.stringify(a));
  const inner = window.__TAURI_INTERNALS__;
  const original = inner.invoke.bind(inner);
  inner.invoke = async (cmd, args, opts) => {
    try {
      const r = await original(cmd, args, opts);
      if (!/skin_signals|get_network_status/.test(cmd)) window.__gvCalls.push(cmd + " -> " + JSON.stringify(r).slice(0, 300));
      return r;
    } catch (e) {
      window.__gvCalls.push(cmd + " !! " + JSON.stringify(e).slice(0, 300));
      throw e;
    }
  };
  for (const level of ["warn", "error"]) {
    const o = console[level].bind(console);
    console[level] = (...args) => { window.__gvCalls.push(level + ": " + args.map(text).join(" ")); o(...args); };
  }
  window.addEventListener("error", (e) => window.__gvCalls.push("uncaught: " + text(e.error ?? e.message)));
  window.addEventListener("unhandledrejection", (e) => window.__gvCalls.push("unhandled rejection: " + text(e.reason)));
}`);
}

/** What `recordIpc` saw since the last read (then cleared), and the toasts now shown. */
async function ipcSeen(b: Browser): Promise<{ calls: string[]; toasts: string[] }> {
  return (await b.execute(() => {
    const w = window as unknown as { __gvCalls?: string[] };
    const calls = w.__gvCalls ? w.__gvCalls.splice(0) : [];
    return { calls, toasts: Array.from(document.querySelectorAll(".toast-item")).map((t) => (t.textContent ?? "").trim()) };
  })) as { calls: string[]; toasts: string[] };
}

describe("a group's voice tab: Call, Join while its call rings, Leave once joined", function () {
  this.timeout(2_400_000);
  const alice = { ...newUser("alice"), displayName: "Alice" };
  const bob = { ...newUser("bob"), displayName: "Bob" };
  let peers: Peer[] = [];
  /** Alice's twin group window. */
  let twinHandle = "";

  before(async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);
    const hello = `gv ${Date.now()}`;
    await requestFriend(a, alice, await friendCode(b), hello);
    await pendingRequest(b, hello);
    await acceptRequest(b, bob, hello);
    await friendRow(a, bob.displayName);
    peers = await startFleet(["Cara"]);
    await befriendFleet(a, alice, peers);
    await befriendFleet(b, bob, peers);
    for (const name of [GROUP, TWIN]) {
      await switchTo(a, BUDDY);
      await createGroup(a, alice, name, [bob.displayName, "Cara"]);
      await peerAcceptGroup(peers[0], name);
      await switchTo(b, BUDDY);
      await press(b, await shown(b, `[aria-label="Accept invite to ${name}"]`, NET_WAIT_MS));
      await resolveIntent(b, { passphrase: bob.passphrase, appearMs: NET_WAIT_MS });
    }
    const [ha, hb] = await Promise.all([symbiosis(a, alice), symbiosis(b, bob)]);
    // The twin's window first, its voice tab's place read; then the group's.
    await openGroup(a, TWIN);
    await skinShown(a, ha);
    twinHandle = await a.getWindowHandle();
    await voiceTab(a, "Alice (twin)", "Call");
    await openGroup(a, GROUP);
    await skinShown(a, ha);
    await openGroup(b, GROUP);
    await skinShown(b, hb);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("with no call, Alice's group window shows Call", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    await voiceTab(a, "Alice", "Call");
    await shot("no-call-call-tab");
  });

  it("Bob calls the group from his Call tab; Alice's window shows Join", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    const call = await voiceTab(b, "Bob", "Call");
    // A disabled Call ignores the press (no IPC, no toast): say so, with the Host's reason.
    if (call.disabled) throw new Error(`Bob's Call (${call.id}) is drawn disabled: ${call.reason || "no reason given"}`);
    await pressNode(b, call.id);
    // GV2: the group's Call opens Start Group Call for this group, in this
    // window, titled with its name and its other members ticked; only they
    // are listed.
    await waitUntil(
      async () => (await (await b.$(".modal-title")).getText().catch(() => "")) === `Call ${GROUP}`,
      NET_WAIT_MS,
      `Bob's Call tab opened no "Call ${GROUP}" panel`,
    );
    const listed = (await b.execute(() =>
      Array.from(document.querySelectorAll(".start-group-call-list label.settings-option")).map((l) => ({
        name: (l.querySelector(".buddy-name")?.textContent ?? "").trim(),
        ticked: (l.querySelector<HTMLInputElement>("input[type=checkbox]")?.checked ?? false),
      })),
    )) as { name: string; ticked: boolean }[];
    console.log(`[gv] Bob's panel: ${JSON.stringify(listed)}`);
    expect(listed.map((x) => x.name).sort()).toEqual([alice.displayName, "Cara"].sort());
    expect(listed.every((x) => x.ticked)).toBe(true);
    await shot("prefilled-panel");
    await pressPanelCall(b, "Bob");
    await voiceTab(a, "Alice", "Join");
    await shot("ringing-join-tab");
    // The twin group, with the same members, does not ring: its tab still reads Call.
    const groupHandle = await a.getWindowHandle();
    await a.switchToWindow(twinHandle);
    await voiceTab(a, "Alice (twin)", "Call");
    expect(await voiceNode(a, "Join")).toBeNull();
    await shot("twin-still-call");
    await a.switchToWindow(groupHandle);
  });

  it("Alice presses Join and her window shows Leave", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const join = await voiceTab(a, "Alice", "Join");
    await pressNode(a, join.id);
    await voiceTab(a, "Alice", "Leave");
    await shot("joined-leave-tab");
  });

  it("Alice presses Leave: she is out of the call (GV3)", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const leave = await voiceTab(a, "Alice", "Leave");
    await pressNode(a, leave.id);
    // Out of the call: Leave is gone. Bob's call goes on, so her window may
    // offer Join again, or Call once it has ended.
    await voiceTabOneOf(a, "Alice (after Leave)", ["Join", "Call"]);
    expect(await voiceNode(a, "Leave")).toBeNull();
    await shot("left-call");
  });

  it("Bob presses Leave: the group's call ends and Alice's window shows Call (GV3)", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    const leave = await voiceTab(b, "Bob", "Leave");
    // The recorder proven on a press that works (Leave) before Cancel's.
    await recordIpc(b);
    await ipcSeen(b);
    await pressNode(b, leave.id);
    await b.pause(3000);
    console.log(`[gv] Bob after Leave (recorder check): ${JSON.stringify(await ipcSeen(b))}`);
    await voiceTab(b, "Bob (after Leave)", "Call");
    await voiceTab(a, "Alice (call ended)", "Call");
    await shot("call-ended");
  });

  it("Bob rings the group again and cancels his ring: Alice's Join goes back to Call (GV3)", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    const call = await voiceTab(b, "Bob", "Call");
    // A disabled Call ignores the press (no IPC, no toast): say so, with the Host's reason.
    if (call.disabled) throw new Error(`Bob's Call (${call.id}) is drawn disabled: ${call.reason || "no reason given"}`);
    await pressNode(b, call.id);
    await waitUntil(
      async () => (await (await b.$(".modal-title")).getText().catch(() => "")) === `Call ${GROUP}`,
      NET_WAIT_MS,
      `Bob's Call tab opened no "Call ${GROUP}" panel`,
    );
    await pressPanelCall(b, "Bob");
    await voiceTab(a, "Alice", "Join");
    // Bob's own ring (groupStarted): the pack's Cancel (cancelVoiceChat), not
    // Leave, which holds only once the call is connected.
    const cancel = await voiceTab(b, "Bob (ringing)", "Cancel");
    expect(await voiceNode(b, "Leave")).toBeNull();
    await shot("own-ring-cancel-tab");
    // What Cancel's press answered on Bob's side (the recorder, proven on Leave above).
    await recordIpc(b);
    await ipcSeen(b);
    // Lead (gv9: no call at all after Cancel): what the press hits. The
    // element at Cancel's centre, and for the click and pointerdown events
    // their target, its nearest listening node ([data-skin-on], what the
    // tree's `mine` compares) and Cancel's served state.
    const atCentre = (await b.execute((id: string) => {
      const n = document.querySelector<HTMLElement>(`[data-skin-node="${id}"]`);
      if (!n) return "no node";
      const r = n.getBoundingClientRect();
      const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) as HTMLElement | null;
      const listener = top?.closest<HTMLElement>("[data-skin-on]");
      (window as unknown as { __gvPress?: string[] }).__gvPress = [];
      for (const kind of ["pointerdown", "click"]) {
        document.addEventListener(
          kind,
          (e) => {
            const t = e.target as HTMLElement | null;
            const l = t?.closest<HTMLElement>("[data-skin-on]");
            (window as unknown as { __gvPress: string[] }).__gvPress.push(
              `${kind}: target ${t?.tagName}.${t?.className} in ${t?.closest<HTMLElement>("[data-skin-node]")?.dataset.skinNode}; listener ${l?.dataset.skinNode} (${l === n ? "Cancel" : "not Cancel"})`,
            );
          },
          { capture: true, once: true },
        );
      }
      return `box ${[r.x, r.y, r.width, r.height].map(Math.round)}; top ${top?.tagName}.${top?.className} in ${top?.closest<HTMLElement>("[data-skin-node]")?.dataset.skinNode}; its listener ${listener?.dataset.skinNode}; Cancel data-state ${n.dataset.state}, aria-disabled ${n.getAttribute("aria-disabled")}, data-skin-on ${n.dataset.skinOn}`;
    }, cancel.id)) as string;
    console.log(`[gv] Cancel before press: ${atCentre}`);
    await pressNode(b, cancel.id);
    await b.pause(3000);
    console.log(`[gv] Cancel press events: ${JSON.stringify(await b.execute(() => (window as unknown as { __gvPress?: string[] }).__gvPress ?? []))}`);
    console.log(`[gv] Bob after Cancel: ${JSON.stringify(await ipcSeen(b))}`);
    await voiceTab(a, "Alice (ring cancelled)", "Call");
    await voiceTab(b, "Bob (ring cancelled)", "Call");
    await shot("ring-cancelled");
  });
});
