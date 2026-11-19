// Plan E7.13: the community [PEER] rows (B61, M82-M88, M2) of
// .claude/plans/standards-remediation/evidence/app-journeys-2026-11-17.md.
// The peer (rows-peer.ts PEER0) joins the walk's community by invite; the
// walk's identity, its owner, moderates it from the member menu
// (MemberList.tsx:165-300; M84 and M85 need the peer in a voice channel, so
// they live in rows-voice.ts, spliced in before M88 by rows.ts); then the walk joins a community the peer creates,
// from the community window's sidebar (CommunitySidebar.tsx:37).

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, present, press, resolveIntent, shown, typeInto, waitUntil } from "./helpers.ts";
import { inBuddy } from "./nav.ts";
import { closeCommunityModal, contextMenu, inCommunity, openCommunitySettings } from "./rows-community.ts";
import { needPeer } from "./rows-peer.ts";
import type { Row } from "./rows.ts";

type El = Awaited<ReturnType<Browser["$"]>>;

const PEER_COMMUNITY = "walk-peer-community";

/** A new invite link from the open community window's settings (row CS25's steps). */
async function createInvite(b: Browser, passphrase: string): Promise<string> {
  await openCommunitySettings(b, "Invites");
  const before = (await b.$$(".invite-link-display")).length;
  await (await shown(b, ".modal-container .settings-content .form-btn-primary")).click();
  await (await byText(b, ".modal-container .form-btn-primary", "Create")).click();
  await resolveIntent(b, { passphrase, appearMs: 3000 });
  await waitUntil(async () => (await b.$$(".invite-link-display")).length > before, NET_WAIT_MS, "no new invite link shown");
  // The link is the read-only input's value; the box's text is only its
  // buttons, "Copy Link" and "Dismiss" (InvitesTab.tsx:126-145).
  const links = await b.$$(".invite-link-display input");
  const link = (await links[links.length - 1].getValue()).trim();
  if (!link.startsWith("rekindle://invite/")) throw new Error(`the new invite shows no link: "${link.slice(0, 40)}"`);
  await closeCommunityModal(b);
  return link;
}

/** Submit an invite in the open Join Community modal (JoinCommunityModal.tsx). */
async function submitJoin(b: Browser, link: string, passphrase: string): Promise<void> {
  await typeInto(await shown(b, 'input[placeholder="rekindle://invite/..."]'), link);
  await (await b.$('.modal-container button[type="submit"]')).click();
  await resolveIntent(b, { passphrase, appearMs: 3000 });
  // A refused join keeps the modal open with the Host's reason in
  // `.form-error` (SimpleInputModal.tsx:47-49, lifecycle.ts:60-68): report it.
  let refused = "";
  await waitUntil(
    async () => {
      const err = await b.$(".modal-container .form-error");
      if ((await err.isExisting()) && (await err.isDisplayed())) refused = await err.getText();
      return refused !== "" || !(await present(b, 'input[placeholder="rekindle://invite/..."]'));
    },
    NET_WAIT_MS,
    'input[placeholder="rekindle://invite/..."] still displayed after the join, with no error',
  );
  if (refused) throw new Error(`join refused: ${refused}`);
}

/** The peer joins the walk's community from its buddy list (B61's steps). */
async function peerJoins(b: Browser, walkPass: string, community: string): Promise<void> {
  const p = needPeer();
  await inCommunity(b, community);
  const link = await createInvite(b, walkPass);
  await inBuddy(p.b);
  await (await shown(p.b, '.buddy-tab[title^="Communities"]')).click();
  await (await shown(p.b, '[aria-label="Join Community"]')).click();
  await submitJoin(p.b, link, p.s.passphrase);
  await byText(p.b, ".community-item", community, NET_WAIT_MS);
}

export async function peerMember(b: Browser): Promise<El> {
  if (!(await present(b, ".member-item"))) await (await shown(b, '[aria-label="Show members panel"]')).click();
  return byText(b, ".member-item", needPeer().s.displayName, NET_WAIT_MS);
}

function memberListed(b: Browser): Promise<boolean> {
  return b.$$(".member-item").then(async (items) => {
    for (const i of items) if ((await i.getText()).includes(needPeer().s.displayName)) return true;
    return false;
  });
}

/** The peer's community window on `community` shows its removal banner (CommunityWindow.tsx `.removal-banner`). */
async function peerSeesRemoval(community: string): Promise<void> {
  const p = needPeer();
  await inCommunity(p.b, community);
  // A community window already open on the peer may show another community.
  await (await byText(p.b, ".community-sidebar .community-item", community, NET_WAIT_MS)).click();
  await shown(p.b, ".removal-banner", NET_WAIT_MS);
}

/** Open the peer's member menu, a submenu (Kobalte SubTrigger) in it, and one of its items. */
async function subItem(b: Browser, trigger: string, item: string, community: string): Promise<void> {
  await b.execute((node: HTMLElement) => {
    const r = node.getBoundingClientRect();
    node.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: r.x + 5, clientY: r.y + 5 }));
  }, (await peerMember(b)) as unknown as HTMLElement);
  await shown(b, ".context-menu");
  const found = await byText(b, ".context-menu .context-menu-item", trigger).catch(async (e: Error) => {
    const items = await b.execute(() =>
      Array.from(document.querySelectorAll(".context-menu .context-menu-item")).map((x) => x.textContent?.trim()).join(" | "),
    );
    throw new Error(`${e.message} (menu: ${items}; ${await rolesProbe(b, community)})`);
  });
  await press(b, found);
  await press(b, await byText(b, ".context-menu .context-menu-item", item));
}

/**
 * The roles the Host gives us and every member of `community` (by name),
 * read in the page when the member menu lacks an item: the menu shows a
 * moderation item only when our highest role outranks the member's and our
 * roles grant the permission (`MemberList.tsx` `canManage`, `myPerms`).
 */
async function rolesProbe(b: Browser, community: string): Promise<string> {
  await b.execute((name: string) => {
    type W = { __walkRoles?: string; __TAURI_INTERNALS__: { invoke(cmd: string, args?: unknown): Promise<unknown> } };
    const w = window as unknown as W;
    const unwrap = (v: unknown): unknown => (v && typeof v === "object" && "value" in v ? (v as { value: unknown }).value : v);
    w.__walkRoles = "pending";
    void w.__TAURI_INTERNALS__.invoke("get_community_details")
      .then(async (d) => {
        const list = unwrap(d) as { id: string; name: string; myRoleIds: string[]; roles: { id: string; name: string; position: number; permissions: string }[] }[];
        const c = list.find((x) => x.name === name);
        if (!c) return `no community named ${name}`;
        const m = unwrap(await w.__TAURI_INTERNALS__.invoke("get_community_members", { communityId: c.id })) as {
          displayName: string;
          roleIds: string[];
        }[];
        const roles = c.roles.map((r) => `${r.id}:${r.name}@${r.position}/${r.permissions}`).join(", ");
        const members = m.map((x) => `${x.displayName}=[${x.roleIds.join(",")}]`).join(", ");
        return `myRoleIds=[${c.myRoleIds.join(",")}] roles={${roles}} members={${members}}`;
      })
      .then((r) => { w.__walkRoles = r; }, (e: unknown) => { w.__walkRoles = `roles probe failed: ${String(e)}`; });
  }, community);
  let got = "pending";
  await waitUntil(
    async () => {
      got = await b.execute(() => (window as unknown as { __walkRoles?: string }).__walkRoles ?? "unset");
      return got !== "pending";
    },
    10_000,
    "roles probe never answered",
  ).catch(() => {});
  return got;
}

/**
 * What the Host's member list and the window's rows hold for `community`
 * (by name), read in the page when the peer is not listed: tells a roster
 * the Host never wrote from a list the window never re-read.
 */
async function memberProbe(b: Browser, community: string): Promise<string> {
  await b.execute((name: string) => {
    type W = { __walkProbe?: string; __TAURI_INTERNALS__: { invoke(cmd: string, args?: unknown): Promise<unknown> } };
    const w = window as unknown as W;
    // A Host call answers `Done<T>` (`{ value, undo }`, ipc/commands/dto.ts).
    const unwrap = (v: unknown): unknown => (v && typeof v === "object" && "value" in v ? (v as { value: unknown }).value : v);
    w.__walkProbe = "pending";
    void w.__TAURI_INTERNALS__.invoke("get_community_details")
      .then(async (d) => {
        const list = unwrap(d) as { id: string; name: string }[];
        const c = list.find((x) => x.name === name);
        if (!c) return `no community named ${name}`;
        const m = unwrap(await w.__TAURI_INTERNALS__.invoke("get_community_members", { communityId: c.id })) as {
          pseudonymKey: string;
          displayName: string;
        }[];
        return `host lists ${m.length}: ${m.map((x) => `${x.displayName || "(no name)"}:${x.pseudonymKey.slice(0, 8)}`).join(", ")}`;
      })
      .then((r) => { w.__walkProbe = r; }, (e: unknown) => { w.__walkProbe = `probe failed: ${String(e)}`; });
  }, community);
  let got = "pending";
  await waitUntil(
    async () => {
      got = await b.execute(() => (window as unknown as { __walkProbe?: string }).__walkProbe ?? "unset");
      return got !== "pending";
    },
    10_000,
    "member probe never answered",
  ).catch(() => {});
  const rows = await b.execute(() => Array.from(document.querySelectorAll(".member-item")).map((e) => e.textContent?.trim()).join(" | "));
  return `${got}; window rows: ${rows}`;
}

export const PEER_COMMUNITY_ROWS: Row[] = [
  {
    id: "B61",
    title: "Join community by invite (peer, buddy list)",
    async run({ b, s, shot }) {
      await peerJoins(b, s.passphrase, s.communityName);
      await needPeer().b.saveScreenshot("e2e/walk/out/B61-peer.png");
      await peerMember(b).catch(async (e: Error) => {
        throw new Error(`${e.message} (${await memberProbe(b, s.communityName)})`);
      });
      await shot();
      return "the walk's member list shows the peer";
    },
  },
  {
    id: "M82",
    title: "Member menu > Manage roles (assign walk-role)",
    needs: ["CS28"],
    async run({ b, s, shot }) {
      await subItem(b, "Manage Roles", "walk-role", s.communityName);
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await b.keys("Escape");
      await waitUntil(async () => (await (await peerMember(b)).getText()).includes("walk-role"), NET_WAIT_MS, "the peer's member row shows no walk-role");
      await shot();
    },
  },
  {
    id: "M83",
    title: "Member menu > Timeout 5 minutes",
    async run({ b, s, shot }) {
      await subItem(b, "Timeout", "5 minutes", s.communityName);
      const outcome = await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await shot();
      if (outcome !== "committed") throw new Error(`timeout intent ${outcome}`);
    },
  },
  {
    id: "M88",
    title: "Member menu > Block, then Unblock",
    async run({ b, s, shot }) {
      await contextMenu(b, await peerMember(b), "Block");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await contextMenu(b, await peerMember(b), "Unblock");
      await shot();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
    },
  },
  {
    id: "M86",
    title: "Member menu > Kick",
    async run({ b, s, shot }) {
      await contextMenu(b, await peerMember(b), "Kick Member");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await waitUntil(async () => !(await memberListed(b)), NET_WAIT_MS, "the kicked peer is still listed");
      // The kick is done when the kicked peer has merged its own removal (as
      // the netsim's rejoin_after_kick waits, "bea merges its own removal"):
      // it learns from the remover's `MemberRemoved` on the DHT
      // (own_removal.rs), seconds after our list drops it. A rejoin before
      // that finds the peer still a current member and answers it without a
      // join (join.rs `already_joined`, RV7 M3): walk 23 M87, whose join ran
      // at 17:04:55, three seconds before the peer merged the removal.
      await peerSeesRemoval(s.communityName);
      await shot();
    },
  },
  {
    id: "M87",
    title: "Member menu > Ban (the peer rejoins first)",
    async run({ b, s, shot }) {
      await peerJoins(b, s.passphrase, s.communityName);
      // Walk 21: the peer's Host logged no join stage for this rejoin; the
      // peer's own Host view says whether it holds the community again.
      const member = await peerMember(b).catch(async (e: Error) => {
        const peerView = await memberProbe(needPeer().b, s.communityName);
        throw new Error(`${e.message} (${await memberProbe(b, s.communityName)}; peer: ${peerView})`);
      });
      await contextMenu(b, member, "Ban Member");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await waitUntil(async () => !(await memberListed(b)), NET_WAIT_MS, "the banned peer is still listed");
      await shot();
    },
  },
  {
    id: "M2",
    title: "Sidebar: Join community (the peer's community)",
    async run({ b, s, shot }) {
      const p = needPeer();
      await inBuddy(p.b);
      await (await shown(p.b, '.buddy-tab[title^="Communities"]')).click();
      await (await shown(p.b, '[aria-label="Create Community"]')).click();
      await typeInto(await shown(p.b, 'input[placeholder="Community name..."]'), PEER_COMMUNITY);
      await (await p.b.$('.modal-container button[type="submit"]')).click();
      await resolveIntent(p.b, { passphrase: p.s.passphrase });
      await inCommunity(p.b, PEER_COMMUNITY);
      // A community window already open on the peer shows another community.
      await (await byText(p.b, ".community-sidebar .community-item", PEER_COMMUNITY, NET_WAIT_MS)).click();
      const link = await createInvite(p.b, p.s.passphrase);
      await inCommunity(b, s.communityName);
      await (await shown(b, '[title="Join Community"]')).click();
      await submitJoin(b, link, s.passphrase);
      await byText(b, ".community-sidebar", PEER_COMMUNITY, NET_WAIT_MS);
      await shot();
      return "the walk's sidebar lists the peer's community";
    },
  },
];
