// Plan E7.2 A (contract A): the skin's Show Voice brings forward what the
// user is talking in. In a 1:1 call it raises the call window; in a community
// voice channel it shows that channel, in a closed community window (opened
// on `?channel=`) and in an open one (CommunityFocusChannel). Both open the
// microphone, so this runs on Linux under wdio.devices.conf.ts and refuses on
// macOS before anything opens it (a TCC check nothing here may trigger). The
// case with neither is specs/solo/show-voice.e2e.ts. The button is a pack
// derived from Symbiosis (skin-pack.ts showVoicePack), imported through
// Settings → Import.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, WAIT_MS, byText, doubleClick, gone, press, resolveIntent, shown, switchTo, typeInto, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings, setValue } from "../../../walk/nav.ts";
import { type Peer, startFleet, stopFleet } from "../../fleet.ts";
import { createCommunity, createIdentity, friendRow, friendsView, importAndKeepSkin, newUser, openCommunity, skinShown, type User } from "../../journeys.ts";
import { SHOW_VOICE_PACK, showVoicePack } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const VOICE = "Lounge";

/** Press the pack's Show Voice button in the buddy list. */
async function pressShowVoice(): Promise<void> {
  await switchTo(browser, BUDDY);
  await friendsView(browser);
  await press(browser, await shown(browser, `[data-skin-node="${SHOW_VOICE_PACK.node}"]`, NET_WAIT_MS));
}

/** The selected channel's name in the community window `browser` is in. */
async function selectedChannel(): Promise<string> {
  return (await browser.execute(() => document.querySelector(".channel-item.channel-item-selected")?.textContent?.trim() ?? "")) as string;
}

describe("Show Voice in a call and in a voice channel", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("showvoicedev"), displayName: "Ari" };
  let peers: Peer[] = [];

  before(async () => {
    if (process.platform === "darwin") throw new Error("macOS: a call or voice channel opens the microphone (a TCC check); run this on Linux");
    await createIdentity(browser, me);
    peers = await startFleet(["Bea"]);
    await befriendFleet(browser, me, peers);
    await openSettings(browser, "Appearance");
    const href = await importAndKeepSkin(browser, me, showVoicePack(), SHOW_VOICE_PACK.name);
    await switchTo(browser, BUDDY);
    await skinShown(browser, href);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("in a 1:1 call, Show Voice brings the call window forward", async () => {
    const before = await browser.getWindowHandles();
    await doubleClick(browser, await friendRow(browser, "Bea"));
    await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("chat-") && !before.includes(h)));
    await press(browser, await shown(browser, '[aria-label="Start voice call with Bea"]', NET_WAIT_MS));
    await shown(browser, ".active-call-panel-header", WAIT_MS * 3);
    const open = await browser.getWindowHandles();
    await pressShowVoice();
    const call = await waitForHandle(browser, (h) => h.startsWith("call-") && !open.includes(h), NET_WAIT_MS);
    await browser.switchToWindow(call);
    await shown(browser, ".active-call-controls");
    await press(browser, await shown(browser, '[aria-label="Hang up"]'));
    await waitUntil(async () => !(await browser.getWindowHandles()).some((h) => h.startsWith("call-")), NET_WAIT_MS, "the call window stayed open after Hang up");
  });

  it("in a voice channel, Show Voice shows it: in the open community window, then in a closed one", async () => {
    await switchTo(browser, BUDDY);
    await createCommunity(browser, me, "Night Raid");
    await openCommunity(browser, "Night Raid");
    // A voice channel, as the walk makes one (rows-voice.ts M10v), joined
    // from the call stage's Join button (CallStage.tsx).
    await (await shown(browser, '[title="Create Channel"]')).click();
    await typeInto(await shown(browser, 'input[placeholder="Channel name..."]'), VOICE);
    await setValue(browser, ".modal-container select.form-select", "voice");
    await (await browser.$('.modal-container button[type="submit"]')).click();
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 3000 });
    await gone(browser, 'input[placeholder="Channel name..."]', NET_WAIT_MS);
    await (await byText(browser, ".channel-item", VOICE, NET_WAIT_MS)).click();
    await (await shown(browser, ".call-stage-join-btn", WAIT_MS)).click();
    await shown(browser, ".voice-panel", NET_WAIT_MS);
    const community = await browser.getWindowHandle();

    // The open window: away to the text channel, then Show Voice brings the
    // voice channel back (CommunityFocusChannel).
    for (const c of await browser.$$(".channel-item")) {
      if ((await c.getText()).trim() === VOICE) continue;
      await c.click();
      break;
    }
    await waitUntil(async () => (await selectedChannel()) !== VOICE, WAIT_MS, "the text channel never became selected");
    await pressShowVoice();
    await browser.switchToWindow(community);
    await waitUntil(async () => (await selectedChannel()) === VOICE, NET_WAIT_MS, `the open window never showed ${VOICE}`);

    // A closed window: Show Voice opens it on the voice channel (?channel=).
    await browser.closeWindow();
    await switchTo(browser, BUDDY);
    const open = await browser.getWindowHandles();
    await pressShowVoice();
    await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("community-") && !open.includes(h), NET_WAIT_MS));
    await waitUntil(async () => (await selectedChannel()) === VOICE, NET_WAIT_MS, `the new window never opened on ${VOICE}`);
    expect(await selectedChannel()).toBe(VOICE);
  });
});
