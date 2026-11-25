// FX3 (lead): the RequestMEK responder gates a restricted channel's key on
// its read roster. Cara (fleet) creates a community; Ari (the app) and Bea
// (fleet) join. Cara makes two channels and restricts both (VIEW denied to
// @everyone): "den" allows Ari, "vault" allows Bea. Cara posts in each.
// Ari, on den's roster, gets its key and reads Cara's post there. Ari, off
// vault's roster, gets no key: vault stays listed with the Host's lock
// (`ChannelView.locked`, its `lockedReason` as the tooltip), its composer
// closed with that reason, and Cara's post never readable. Each state is
// logged as `[fx3]` and shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerJoinCommunity, peerMemberCount, peerPostChannel, peerReviewed, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, newUser, openCommunity, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Roster Hall ${Date.now() % 100000}`;
/** `rekindle_types::permissions::VIEW_CHANNELS` (1 << 0). */
const VIEW = "0x1";
const DEN_POST = `den post ${Date.now()}`;
const VAULT_POST = `vault post ${Date.now()}`;

/** `peer`'s pseudonym in `anchor` (community info's `ourPseudonym`). */
async function peerPseudonym(peer: Peer, anchor: string): Promise<string> {
  const info = (await peer.cli(["community", "info", "--community", anchor])) as { ourPseudonym?: string; our_pseudonym?: string } | null;
  const key = info?.ourPseudonym ?? info?.our_pseudonym ?? "";
  if (!/^[0-9a-f]{64}$/i.test(key)) throw new Error(`${peer.displayName}: community info served no pseudonym (${JSON.stringify(info).slice(0, 200)})`);
  return key;
}

/** Ari's pseudonym in `anchor`, as the Host lists the community. */
async function ourPseudonym(anchor: string): Promise<string> {
  let key = "";
  await waitUntil(
    async () => {
      key = (await browser.execute(async (a: string) => {
        const w = window as unknown as { __TAURI_INTERNALS__?: { invoke: (cmd: string) => Promise<unknown> } };
        const done = (await w.__TAURI_INTERNALS__!.invoke("get_communities")) as { value?: { anchor: string; ourPseudonym: string }[] };
        return done.value?.find((c) => c.anchor === a)?.ourPseudonym ?? "";
      }, anchor)) as string;
      return /^[0-9a-f]{64}$/i.test(key);
    },
    NET_WAIT_MS,
    "Ari's pseudonym was never served",
  );
  return key;
}

/** Channel `name`'s 32-hex id in `anchor`, from `peer`'s channel list. */
async function channelId(peer: Peer, anchor: string, name: string): Promise<string> {
  const found: string[] = [];
  const walk = (v: unknown): void => {
    if (Array.isArray(v)) v.forEach(walk);
    else if (v && typeof v === "object") {
      const o = v as Record<string, unknown>;
      if (o.name === name && typeof o.id === "string") found.push(o.id);
      Object.values(o).forEach(walk);
    }
  };
  await waitUntil(
    async () => {
      walk(await peer.cli(["channel", "list", "-c", anchor]));
      return found.length > 0;
    },
    NET_WAIT_MS,
    `${peer.displayName} never listed channel ${name}`,
  );
  return found[0];
}

/** Cara restricts `channel` to `member` (VIEW denied to @everyone, allowed to the member). */
async function restrict(cara: Peer, anchor: string, channel: string, member: string): Promise<void> {
  await peerReviewed(cara, ["community", "channel", "overwrite", "set", "-c", anchor, "-C", channel, "--target", "everyone", "--deny", VIEW]);
  await peerReviewed(cara, ["community", "channel", "overwrite", "set", "-c", anchor, "-C", channel, "--target", `member:${member}`, "--allow", VIEW]);
}

/** The open community window's channel row `name`: listed, locked (with its words), and what the open channel shows. */
async function channelState(name: string): Promise<{ listed: boolean; locked: string | null; composer: string; text: string }> {
  return (await browser.execute((n: string) => {
    const row = Array.from(document.querySelectorAll(".channel-item")).find((r) => (r.querySelector(".channel-name")?.textContent ?? "").trim() === n);
    const lock = row?.querySelector<HTMLElement>(".channel-locked");
    return {
      listed: !!row,
      locked: lock ? (lock.getAttribute("title") ?? "") : null,
      composer: (document.querySelector(".typing-indicator .typing-label")?.textContent ?? "").trim(),
      text: (document.querySelector("#main-content")?.textContent ?? "").replace(/\s+/g, " "),
    };
  }, name)) as { listed: boolean; locked: string | null; composer: string; text: string };
}

describe("a restricted channel's key goes only to its roster (FX3)", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("fx3"), displayName: "Ari" };
  let peers: Peer[] = [];
  let anchor = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara", "Bea"]);
    const [cara, bea] = peers;
    anchor = await peerCreateCommunity(cara, COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(cara, anchor, 1), COMMUNITY);
    await peerJoinCommunity(bea, await peerInvite(cara, anchor, 1));
    await waitUntil(async () => (await peerMemberCount(cara, anchor)) >= 3, NET_WAIT_MS * 2, "Cara never counted three members");
    await peerReviewed(cara, ["channel", "create", "-c", anchor, "-n", "den"]);
    await peerReviewed(cara, ["channel", "create", "-c", anchor, "-n", "vault"]);
    const den = await channelId(cara, anchor, "den");
    const vault = await channelId(cara, anchor, "vault");
    await restrict(cara, anchor, den, await ourPseudonym(anchor));
    await restrict(cara, anchor, vault, await peerPseudonym(bea, anchor));
    await peerPostChannel(cara, anchor, den, DEN_POST);
    await peerPostChannel(cara, anchor, vault, VAULT_POST);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("on den's roster, Ari gets its key and reads Cara's post", async () => {
    await openCommunity(browser, COMMUNITY);
    await selectChannel(browser, "den");
    let s = await channelState("den");
    await waitUntil(async () => (s = await channelState("den")).text.includes(DEN_POST), NET_WAIT_MS * 2, "").catch(() => undefined);
    console.log(`[fx3] den: ${JSON.stringify({ ...s, text: s.text.slice(0, 200) })}`);
    await shot("den-readable");
    expect(s.locked).toBeNull();
    expect(s.text).toContain(DEN_POST);
  });

  it("off vault's roster, Ari gets no key: vault is listed with the Host's lock and its composer closed", async () => {
    let s = await channelState("vault");
    await waitUntil(async () => (s = await channelState("vault")).locked !== null, NET_WAIT_MS * 2, "").catch(() => undefined);
    console.log(`[fx3] vault row: ${JSON.stringify({ ...s, text: "" })}`);
    expect(s.listed).toBe(true);
    expect(s.locked).not.toBeNull();
    expect((s.locked ?? "").length).toBeGreaterThan(0);
    await selectChannel(browser, "vault");
    await browser.pause(5000);
    s = await channelState("vault");
    console.log(`[fx3] vault open: ${JSON.stringify({ ...s, text: s.text.slice(0, 200) })}`);
    await shot("vault-locked");
    expect(s.composer).toBe(s.locked);
    expect(s.text).not.toContain(VAULT_POST);
  });
});
