// Plan E7.13: the [PEER] rows of
// .claude/plans/standards-remediation/evidence/app-journeys-2026-11-17.md.
// A second app (launch.ts `launch("peer")`: its own temp data root, port,
// token and Veilid node) carries a second throwaway identity; the walk's
// identity befriends it by friend code, then they exchange a DM.

import { randomBytes } from "node:crypto";
import { remote, type Browser } from "webdriverio";
import { NET_WAIT_MS, Skip, WAIT_MS, byText, doubleClick, gone, press, resolveIntent, shown, switchTo, typeInto, waitForHandle, waitUntil } from "./helpers.ts";
import { launch, type App } from "./launch.ts";
import { BUDDY, inBuddy } from "./nav.ts";
import { createIdentity, openAddFriend, type Row, type WalkState } from "./rows.ts";

export interface Peer {
  app: App;
  b: Browser;
  s: WalkState;
}

let peer: Peer | null = null;

export async function stopPeer(): Promise<void> {
  if (!peer) return;
  const p = peer;
  peer = null;
  if (p.app.alive()) await p.b.deleteSession().catch(() => undefined);
  await p.app.stop();
}

function needPeer(): Peer {
  if (!peer) throw new Skip("no peer: PEER0 did not start one");
  return peer;
}

async function friendCode(b: Browser): Promise<string> {
  await openAddFriend(b);
  await (await byText(b, ".form-tab", "Public Key")).click();
  return (await (await shown(b, "output.sec-friend-code-url", NET_WAIT_MS)).getText()).trim();
}

async function closeAddFriend(b: Browser): Promise<void> {
  await (await b.$(".modal-close-btn")).click();
  await gone(b, ".modal-container");
}

async function friendItem(b: Browser, name: string): Promise<Awaited<ReturnType<Browser["$"]>>> {
  await inBuddy(b);
  await (await b.$('.buddy-tab[title^="Friends"]')).click();
  return byText(b, ".buddy-item", name, NET_WAIT_MS);
}

async function openChat(b: Browser, name: string): Promise<void> {
  const before = await b.getWindowHandles();
  await doubleClick(b, await friendItem(b, name));
  await b.switchToWindow(await waitForHandle(b, (h) => h.startsWith("chat-") && !before.includes(h), NET_WAIT_MS));
  await shown(b, ".message-input-field", NET_WAIT_MS);
}

const DM = "dm from the walk";

export const PEER_ROWS: Row[] = [
  {
    id: "PEER0",
    title: "Second app on its own data root and port, second identity",
    async run({ b, app, shot }) {
      const papp = await launch("peer");
      const pb = await remote({
        hostname: "127.0.0.1",
        port: papp.port,
        path: "/",
        protocol: "http",
        logLevel: "error",
        connectionRetryCount: 1,
        headers: { Authorization: `Bearer ${papp.token}` },
        capabilities: {
          "wdio:enforceWebDriverClassic": true,
          "wdio:tauriServiceOptions": { windowLabel: "login" },
        } as WebdriverIO.Capabilities,
      });
      const ps: WalkState = {
        displayName: `peer-${randomBytes(3).toString("hex")}`,
        passphrase: randomBytes(18).toString("base64url"),
        communityName: "",
      };
      peer = { app: papp, b: pb, s: ps };
      await createIdentity(pb, ps);
      await pb.saveScreenshot("e2e/walk/out/PEER0-peer.png");
      await shot();
      if (!app.alive()) throw new Error("the walk's app exited when the peer started");
      return `peer ${ps.displayName} on port ${papp.port}, data root ${papp.dataRoot}`;
    },
  },
  {
    id: "B57",
    title: "Add friend by code + message",
    async run({ b, s, shot }) {
      const p = needPeer();
      const code = await friendCode(p.b);
      await closeAddFriend(p.b);
      await inBuddy(b);
      await openAddFriend(b);
      await (await byText(b, ".form-tab", "Public Key")).click();
      await typeInto(await shown(b, 'input[aria-label="Friend code"]'), code);
      await typeInto(await shown(b, 'input[aria-label="Message (optional)"]'), "hi from the walk");
      await (await b.$('.modal-container form button[type="submit"]')).click();
      await resolveIntent(b, { passphrase: s.passphrase });
      await shot();
      await closeAddFriend(b).catch(() => undefined);
      await switchTo(p.b, BUDDY);
      const item = await byText(p.b, ".pending-item", "hi from the walk", NET_WAIT_MS);
      return `peer shows the request: ${(await item.getText()).replace(/\s+/g, " ").slice(0, 80)}`;
    },
  },
  {
    id: "B23",
    title: "Accept friend request (peer)",
    async run({ b, s, shot }) {
      const p = needPeer();
      await (await shown(p.b, ".pending-item .pending-btn-accept", NET_WAIT_MS)).click();
      await resolveIntent(p.b, { passphrase: p.s.passphrase });
      await friendItem(p.b, s.displayName);
      await p.b.saveScreenshot("e2e/walk/out/B23-peer.png");
      await friendItem(b, p.s.displayName);
      await shot();
      return "each buddy list lists the other";
    },
  },
  {
    id: "C12",
    title: "Open DM window loads history",
    async run({ b, shot }) {
      const p = needPeer();
      await openChat(b, p.s.displayName);
      await shot();
    },
  },
  {
    id: "C1",
    title: "Toggle safety-number panel",
    async run({ b, shot }) {
      needPeer();
      await (await shown(b, ".chat-safety-btn")).click();
      await shown(b, "section.sec-safety-number", NET_WAIT_MS);
      await shot();
      await (await b.$(".chat-safety-btn")).click();
      await gone(b, ".sec-safety-number");
    },
  },
  {
    id: "C6",
    title: "Send with Enter; peer receives",
    async run({ b, s, shot }) {
      const p = needPeer();
      await typeInto(await shown(b, ".message-input-field"), DM);
      await b.keys("Enter");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await byText(b, ".chat-message", DM, NET_WAIT_MS);
      await shot();
      await openChat(p.b, s.displayName);
      await byText(p.b, ".chat-message", DM, NET_WAIT_MS);
      await p.b.saveScreenshot("e2e/walk/out/C6-peer.png");
      return "peer's chat window shows the message";
    },
  },
  {
    id: "C7",
    title: "Typing indicator while drafting (seen by the peer)",
    async run({ b, shot }) {
      const p = needPeer();
      await (await shown(b, ".message-input-field")).addValue("typing…");
      await waitUntil(async () => /typing/i.test(await (await p.b.$("body")).getText()), WAIT_MS * 2, "peer saw no typing indicator");
      await p.b.saveScreenshot("e2e/walk/out/C7-peer.png");
      await shot();
      await (await b.$(".message-input-field")).clearValue();
      return "peer shows typing";
    },
  },
  {
    id: "P1",
    title: "Profile: Send Message",
    async run({ b, shot }) {
      const p = needPeer();
      await b.closeWindow();
      const item = await friendItem(b, p.s.displayName);
      await b.execute((node: HTMLElement) => {
        const r = node.getBoundingClientRect();
        node.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: r.x + 5, clientY: r.y + 5 }));
      }, item as unknown as HTMLElement);
      await press(b, await byText(b, ".context-menu-item", "View Profile"));
      await b.switchToWindow(await waitForHandle(b, (h) => h.startsWith("profile-"), NET_WAIT_MS));
      await shown(b, ".profile-actions", NET_WAIT_MS);
      await shot();
      await (await byText(b, ".profile-actions button", "Send Message")).click();
      await waitForHandle(b, (h) => h.startsWith("chat-"), NET_WAIT_MS);
      return "chat window opened from the profile";
    },
  },
  {
    id: "P5",
    title: "Profile: Remove, then Cancel",
    async run({ b, shot }) {
      needPeer();
      await b.switchToWindow(await waitForHandle(b, (h) => h.startsWith("profile-")));
      await (await byText(b, ".profile-actions button", "Remove Friend")).click();
      await byText(b, ".profile-actions button", "Confirm Remove");
      await (await byText(b, ".profile-actions button", "Cancel")).click();
      await byText(b, ".profile-actions button", "Remove Friend");
      await shot();
      await b.closeWindow();
      await inBuddy(b);
    },
  },
  {
    id: "B58",
    title: "New group DM",
    async run() {
      throw new Skip("a group DM has 3-8 members (plan E2.6): two friends; the walk runs one peer");
    },
  },
];
