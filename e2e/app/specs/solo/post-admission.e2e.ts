// CP1 (contract-cp1-2026-11-27): a channel post is admitted at the governance
// cut it signs. Cara (fleet) creates a community; Ari (the app) and Bea
// (fleet) join. Two journeys, each in the Host's words only:
//  - pending: Ari holds incoming governance ops (the webdriver-test aid
//    `holdGovernanceOps`, the netsim's withheld op); Cara writes one (a new
//    channel) and posts in a channel Ari has open, citing it. Ari shows the
//    post as a body-free placeholder with the Host's pending text. Released
//    (the normal store and rebuild), the same message id becomes Cara's post:
//    one row, no placeholder, no duplicate.
//  - refused: Cara denies SEND_MESSAGES to @everyone in a channel; Bea posts
//    there skipping only her own local gate (the adversarial client the
//    reader check exists for, `postSkippingLocalPermission`), signing her true
//    cut. Ari shows the Host's refusal text in the refused tone, no body.
// Each state is logged as `[cp1]` and shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { holdGovernanceOps, postSkippingLocalPermission, releaseGovernanceOps } from "../../cp1.ts";
import {
  type Peer,
  peerCreateCommunity,
  peerInvite,
  peerJoinCommunity,
  peerMemberCount,
  peerPostChannel,
  peerReviewed,
  startFleet,
  stopFleet,
} from "../../fleet.ts";
import {
  createIdentity,
  joinCommunity,
  newUser,
  openCommunity,
  selectChannel,
  type User,
} from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Cut Hall ${Date.now() % 100000}`;
/** `rekindle_types::permissions::SEND_MESSAGES` (bit 16). */
const SEND_MESSAGES = "0x10000";
const PENDING_BODY = `held post ${Date.now()}`;
const REFUSED_BODY = `refused post ${Date.now()}`;
/** The Host's words (rekindle-types dto/channel_message.rs, PostAdmission). */
const PENDING_TEXT = "Waiting to verify that this post was allowed.";
const REFUSED_TEXT =
  "This post was refused because the author did not have permission to post here.";

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
    `${peer.displayName} never listed channel ${name}`
  );
  return found[0];
}

/** The open channel's rows: each message's id, whether it is a held placeholder and its words. */
async function rows(): Promise<{ id: string; held: string | null; text: string }[]> {
  return (await browser.execute(() =>
    Array.from(document.querySelectorAll<HTMLElement>("#main-content .chat-message")).map((m) => {
      const held = m.querySelector<HTMLElement>(".message-held");
      return {
        id: m.getAttribute("data-message-id") ?? "",
        held: held ? (held.getAttribute("data-admission") ?? "") : null,
        text: (m.textContent ?? "").replace(/\s+/g, " ").trim(),
      };
    })
  )) as { id: string; held: string | null; text: string }[];
}

describe("a channel post is admitted at its signed governance cut (CP1)", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("cp1"), displayName: "Ari" };
  let peers: Peer[] = [];
  let anchor = "";
  let heldChannel = "";
  let refusedChannel = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara", "Bea"]);
    const [cara, bea] = peers;
    anchor = await peerCreateCommunity(cara, COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(cara, anchor, 1), COMMUNITY);
    await peerJoinCommunity(bea, await peerInvite(cara, anchor, 1));
    await waitUntil(
      async () => (await peerMemberCount(cara, anchor)) >= 3,
      NET_WAIT_MS * 2,
      "Cara never counted three members"
    );
    await peerReviewed(cara, ["channel", "create", "-c", anchor, "-n", "held"]);
    await peerReviewed(cara, ["channel", "create", "-c", anchor, "-n", "refused"]);
    heldChannel = await channelId(cara, anchor, "held");
    refusedChannel = await channelId(cara, anchor, "refused");
    console.log(`[cp1] community ${anchor.slice(0, 12)}…, channels held and refused`);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("pending: a post citing an op the app has not yet is a placeholder in the Host's words, then the same id becomes the post", async () => {
    const [cara] = peers;
    await openCommunity(browser, COMMUNITY);
    await selectChannel(browser, "held");
    await holdGovernanceOps(browser, anchor);
    // Cara's op (a new channel) is held on Ari; her post signs a cut citing it.
    await peerReviewed(cara, ["channel", "create", "-c", anchor, "-n", "later"]);
    await peerPostChannel(cara, anchor, heldChannel, PENDING_BODY);
    let held = await rows();
    await waitUntil(
      async () => (held = await rows()).some((r) => r.held === "pending"),
      NET_WAIT_MS * 2,
      ""
    ).catch(() => undefined);
    const pending = held.find((r) => r.held === "pending");
    console.log(`[cp1] pending: ${JSON.stringify(pending ?? held)}`);
    await shot("cp1-pending");
    expect(pending).toBeDefined();
    expect(pending!.text).toContain(PENDING_TEXT);
    expect(pending!.text).not.toContain(PENDING_BODY);
    expect(pending!.id).not.toBe("");

    const released = await releaseGovernanceOps(browser, anchor);
    console.log(`[cp1] released ${released} held op(s)`);
    expect(released).toBeGreaterThan(0);
    let after = await rows();
    await waitUntil(
      async () => {
        after = await rows();
        return after.some(
          (r) => r.id === pending!.id && r.held === null && r.text.includes(PENDING_BODY)
        );
      },
      NET_WAIT_MS * 2,
      ""
    ).catch(() => undefined);
    const sameId = after.filter((r) => r.id === pending!.id);
    console.log(
      `[cp1] after release: ${JSON.stringify(sameId)}; rows with the body: ${after.filter((r) => r.text.includes(PENDING_BODY)).length}`
    );
    await shot("cp1-admitted");
    expect(sameId.length).toBe(1);
    expect(sameId[0].held).toBeNull();
    expect(sameId[0].text).toContain(PENDING_BODY);
    expect(after.filter((r) => r.text.includes(PENDING_BODY)).length).toBe(1);
    expect(after.some((r) => r.held === "pending")).toBe(false);
  });

  it("refused: a post its author had no permission for shows the Host's reason, never its body", async () => {
    const [cara, bea] = peers;
    await peerReviewed(cara, [
      "community",
      "channel",
      "overwrite",
      "set",
      "-c",
      anchor,
      "-C",
      refusedChannel,
      "--target",
      "everyone",
      "--deny",
      SEND_MESSAGES,
    ]);
    // The post is judged at the cut its author signs (CP1): Bea's must
    // already hold the deny, or her post is rightly admitted. Her own Host
    // refusing a normal send proves it (there is no read of her Send
    // Messages; a probe sent before the deny arrives is admitted, harmless).
    let probes = 0;
    await waitUntil(
      async () => {
        probes += 1;
        try {
          await peerPostChannel(bea, anchor, refusedChannel, `send probe ${probes}`);
          return false;
        } catch (e) {
          const refusal = e instanceof Error ? e.message : String(e);
          if (!/permission|Send Messages|not allowed|refus/i.test(refusal)) throw e;
          console.log(`[cp1] Bea's own Host refuses a normal send after ${probes} probe(s): ${refusal.slice(0, 160)}`);
          return true;
        }
      },
      NET_WAIT_MS * 2,
      "Bea's own Host never refused a normal send: the deny overwrite never reached her"
    );
    await openCommunity(browser, COMMUNITY);
    await selectChannel(browser, "refused");
    // Bea's own Host refuses to send; the aid skips only that gate.
    await postSkippingLocalPermission(bea, refusedChannel, REFUSED_BODY);
    let seen = await rows();
    await waitUntil(
      async () => (seen = await rows()).some((r) => r.held === "refused"),
      NET_WAIT_MS * 2,
      ""
    ).catch(() => undefined);
    const refused = seen.find((r) => r.held === "refused");
    // The refused tone is the theme's danger text: the row's computed colour
    // must equal --color-xfire-danger-text resolved in the same document.
    const tone = (await browser.execute(() => {
      const el = document.querySelector<HTMLElement>('.message-held[data-admission="refused"]');
      const probe = document.createElement("span");
      probe.style.color = "var(--color-xfire-danger-text)";
      (el?.parentElement ?? document.body).appendChild(probe);
      const danger = getComputedStyle(probe).color;
      probe.remove();
      return { row: el ? getComputedStyle(el).color : "", danger };
    })) as { row: string; danger: string };
    console.log(
      `[cp1] refused: ${JSON.stringify(refused ?? seen)}; colour ${tone.row}, danger ${tone.danger}`
    );
    await shot("cp1-refused");
    expect(refused).toBeDefined();
    expect(refused!.text).toContain(REFUSED_TEXT);
    expect(refused!.text).not.toContain(REFUSED_BODY);
    expect(tone.row).not.toBe("");
    expect(tone.row).toBe(tone.danger);
  });
});
