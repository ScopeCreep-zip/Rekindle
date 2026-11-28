// Probe (lead, 2026-11-26; the "MEMBERS — 0" diagnosis): the creator of a
// community, with #general open, is not listed in the member panel. This asks
// the Host directly, with the same Tauri commands the window uses, what it
// serves: `get_community_details` (the community's id and our own pseudonym)
// and `get_community_members` (the member set and each member's status),
// every 2 s for 30 s, beside the panel's own header and `get_communities`'
// memberCount (the merged governance's count, independent of the roster):
// count 0 with no members = the merge never admitted the creator; count 1
// with no members = the creator's roster row is missing. If the Host lists the
// creator and the panel does not, the shell dropped it; if the Host lists
// nobody, the cause is on the Host's side.

import { browser } from "@wdio/globals";
import { NET_WAIT_MS, byText, communityWindowLabel, doubleClick, shown, switchTo } from "../../../walk/helpers.ts";
import { inBuddy } from "../../../walk/nav.ts";
import { communitiesTab, createCommunity, createIdentity, newUser, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Members Probe ${Date.now() % 100000}`;

interface Served {
  header: string;
  community: string | null;
  me: string | null;
  /** `get_communities`' memberCount: the merged governance's count (GovernanceState::member_count), independent of the SQLite roster. */
  governanceCount: number | null;
  members: { key: string; name: string; status: string }[] | string;
}

async function served(name: string): Promise<Served> {
  return (await browser.execute(async (communityName: string): Promise<Served> => {
    const call = (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown) => Promise<{ value: unknown }> } }).__TAURI_INTERNALS__.invoke;
    const header = document.querySelector(".member-list-header")?.textContent?.trim() ?? "(no header)";
    try {
      const details = (await call("get_community_details")).value as { id: string; name: string; myPseudonymKey: string | null }[];
      const c = details.find((d) => d.name === communityName);
      if (!c) return { header, community: null, me: null, governanceCount: null, members: "community not in get_community_details" };
      // get_communities keys each community by its anchor (the id details use).
      const list = (await call("get_communities")).value as { anchor: string; memberCount: number }[];
      const governanceCount = list.find((x) => x.anchor === c.id)?.memberCount ?? null;
      const members = (await call("get_community_members", { communityId: c.id })).value as { pseudonymKey: string; displayName: string; status: string }[];
      return {
        header,
        community: c.id,
        me: c.myPseudonymKey,
        governanceCount,
        members: members.map((m) => ({ key: m.pseudonymKey.slice(0, 12), name: m.displayName, status: m.status })),
      };
    } catch (e) {
      return { header, community: null, me: null, governanceCount: null, members: `error: ${e instanceof Error ? e.message : JSON.stringify(e)}` };
    }
  }, name)) as Served;
}

describe("MEMBERS — 0 probe: what the Host serves for a new community's creator", function () {
  this.timeout(600_000);
  const me: User = { ...newUser("members"), displayName: "Ari" };

  it("logs the Host's member set beside the panel's header for 30 s", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, COMMUNITY);
    // openCommunity's own steps, so the panel is watched from the moment the
    // window is switched to (it waits for the sidebar first): every change of
    // the member panel's header and row count, timed from the page's load.
    await inBuddy(browser);
    const label = await communityWindowLabel(browser, COMMUNITY);
    await communitiesTab(browser);
    await doubleClick(browser, await byText(browser, ".community-item", COMMUNITY.slice(0, 24), NET_WAIT_MS));
    await switchTo(browser, label, NET_WAIT_MS);
    await browser.execute(() => {
      const log: string[] = [];
      (window as unknown as { __panel: string[] }).__panel = log;
      const read = (): string => {
        const header = document.querySelector(".member-list-header")?.textContent?.trim() ?? "(no header)";
        const rows = document.querySelectorAll(".member-item[data-pseudonym]").length;
        return `${header}, ${rows} rows`;
      };
      let last = read();
      log.push(`${Math.round(performance.now())}ms since load: attached, ${last}`);
      new MutationObserver(() => {
        const now = read();
        if (now !== last) log.push(`${Math.round(performance.now())}ms since load: ${now}`);
        last = now;
      }).observe(document.body, { childList: true, subtree: true, characterData: true });
    });
    await shown(browser, ".community-sidebar", NET_WAIT_MS);
    await selectChannel(browser, "general");
    await shown(browser, ".member-list-header", NET_WAIT_MS);
    let last = "";
    for (let i = 0; i < 15; i++) {
      const s = await served(COMMUNITY);
      const line = JSON.stringify({ ...s, me: s.me?.slice(0, 12) ?? null });
      if (line !== last) console.log(`[members] t+${i * 2}s ${line.slice(0, 600)}`);
      last = line;
      await browser.pause(2000);
    }
    await shot("members-panel");
    const timeline = (await browser.execute(() => (window as unknown as { __panel?: string[] }).__panel ?? [])) as string[];
    for (const line of timeline) console.log(`[members] panel ${line}`);
  });
});
