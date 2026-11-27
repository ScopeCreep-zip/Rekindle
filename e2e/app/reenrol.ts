// The steps a device re-enrolment spec drives (plan E3.2 S7d-keys.9/.10):
// Ari's locked data root copied to a second daemon, the copy left behind its
// own record by an op of Ari's, and the copy's CLI re-enrolment committed;
// then the original's "Signing stopped" alert. Shared by device-reenrol (a
// member carries the retirement) and device-reenrol-solo (the original's own
// record does, S7d-keys.9p2).

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, gone, resolveIntent, shown, switchTo, typeInto, waitUntil } from "../walk/helpers.ts";
import { LOGIN, inBuddy } from "../walk/nav.ts";
import { type Peer, peerReviewed, startClone } from "./fleet.ts";
import { logOut, openCommunity, type User } from "./journeys.ts";
import { showSkinView } from "./journeys_skins.ts";

/** The CLI subcommand that commits ReEnrolDevice on the copy (.10 part 2). */
const REENROL = ["community", "re-enrol-device"];
/** The channel the original creates after the copy: the governance op the copy lacks. */
const AFTER_COPY = "after-the-copy";
/** Long enough for the original's op to reach its own record before the copy unlocks again. */
const PUBLISH_MS = 30_000;

/** Unlock `user` from the login window. */
export async function unlock(b: Browser, user: User): Promise<void> {
  await switchTo(b, LOGIN, NET_WAIT_MS);
  await typeInto(await shown(b, 'form.login-container input[type="password"]'), user.passphrase);
  const submit = await b.$('form.login-container button[type="submit"]');
  await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Unlock stayed disabled (Connecting…)" });
  await submit.click();
}

/**
 * Copy `user`'s root (locked, as the netsim locks the node before its
 * snapshot: copying the live one raced Veilid's own files) to a second
 * daemon; while it is offline the original commits an op of her own (a new
 * channel), so the copy is behind its own record; at its next unlock its Host
 * finds the copy, and its CLI re-enrolment is committed. A copy matching its
 * record has nothing to re-enrol ("this device's key needs no
 * re-enrolment"), so that refusal is retried until the Host finds the copy.
 * Returns the copy; Ari is unlocked in `community`'s window.
 */
export async function copyDivergesAndReEnrols(b: Browser, user: User, community: string, copyName: string): Promise<Peer> {
  const dataRoot = (b as unknown as { requestedCapabilities: Record<string, unknown> }).requestedCapabilities["rekindle:dataRoot"];
  if (typeof dataRoot !== "string") throw new Error("the launcher handed no rekindle:dataRoot capability");
  await logOut(b, user);
  const copy = await startClone(copyName, { dataRoot, passphrase: user.passphrase });
  await unlock(b, user);
  await inBuddy(b);
  await openCommunity(b, community);
  const communities = (await copy.cli(["community", "list"])) as { name?: string }[] | null;
  if (!(communities ?? []).some((c) => c.name === community)) throw new Error(`the copy holds no ${community}: ${JSON.stringify(communities).slice(0, 200)}`);

  await copy.pause();
  await (await shown(b, '[title="Create Channel"]')).click();
  await typeInto(await shown(b, 'input[placeholder="Channel name..."]'), AFTER_COPY);
  await (await b.$('.modal-container button[type="submit"]')).click();
  await resolveIntent(b, { passphrase: user.passphrase, appearMs: 3000 });
  await gone(b, 'input[placeholder="Channel name..."]', NET_WAIT_MS);
  await byText(b, ".channel-item", AFTER_COPY, NET_WAIT_MS);
  await b.pause(PUBLISH_MS);
  await copy.resume();
  let refusal = "";
  await waitUntil(
    async () => {
      try {
        await peerReviewed(copy, REENROL);
        return true;
      } catch (e) {
        refusal = e instanceof Error ? e.message : String(e);
        if (!refusal.includes("needs no re-enrolment")) throw e;
        return false;
      }
    },
    NET_WAIT_MS * 2,
    "",
  ).catch((e) => {
    throw new Error(`the copy never found itself behind its own record: ${refusal || (e instanceof Error ? e.message : String(e))}`);
  });
  console.log("[reenrol] the copy's re-enrolment was accepted");
  return copy;
}

/** The original's "Signing stopped" alert, as the Host serves it, in the buddy list's notifications; its text. */
export async function signingStoppedShown(b: Browser): Promise<string> {
  await inBuddy(b);
  await showSkinView(b, "notifications");
  const alert = await byText(b, "[class*=notification]", "Signing stopped", NET_WAIT_MS * 2);
  return (await alert.getText()).replace(/\s+/g, " ");
}
