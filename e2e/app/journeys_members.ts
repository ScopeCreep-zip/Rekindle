// The app user's own identity inside a community, read from what the Host
// serves (plan E3.2 CAPR specs): split from journeys.ts, which holds the
// cross-window journeys.

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, shown, waitUntil } from "../walk/helpers.ts";
import { closeModal } from "../walk/nav.ts";
import { communitySettings } from "./journeys.ts";

/**
 * Our own pseudonym in the open community window, for the CLI's member
 * commands (they take a pseudonym). A member with no per-community name is
 * listed by fingerprint, not by display name, so the row is found by what
 * the Host serves about us: Settings → Overview's "Members here see you as
 * …" (MyNameSection.tsx, the community's `myDisplayName`), matched to the
 * member row (MemberList.tsx `data-pseudonym`) that shows the same name.
 */
export async function ownCommunityPseudonym(b: Browser): Promise<string> {
  await communitySettings(b, "Overview");
  const hint = await shown(b, "#my-name-hint", NET_WAIT_MS);
  const said = (await hint.getText()).replace(/\s+/g, " ").trim();
  const seenAs = /^Members here see you as (.+)\.$/.exec(said)?.[1];
  await closeModal(b);
  if (!seenAs) throw new Error(`the Overview did not say how members see us: "${said}"`);
  let key = "";
  await waitUntil(
    async () => {
      key = (await b.execute((name: string) => {
        const rows = Array.from(document.querySelectorAll<HTMLElement>(".member-item[data-pseudonym]")).filter(
          (row) => row.querySelector(".member-name")?.textContent?.replace(/\s+/g, " ").trim() === name,
        );
        return rows.length === 1 ? (rows[0].dataset.pseudonym ?? "") : "";
      }, seenAs)) as string;
      return /^[0-9a-f]{64}$/i.test(key);
    },
    NET_WAIT_MS,
    `no single member row shows "${seenAs}" (how the Overview says members see us) with a pseudonym`,
  );
  return key;
}
