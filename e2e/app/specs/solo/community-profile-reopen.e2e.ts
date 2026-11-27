// PF2 uses the real community window and Host. The native file dialog is
// supplied by pickNextFile so the uploaded images are reproducible.
import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, gone, press, resolveIntent, shown, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { pickNextFile } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, newUser, openCommunity, selectChannel } from "../../journeys.ts";
import { decodePng, encodePng } from "../../png.ts";
import { saveImage, shot } from "../../shots.ts";
import type { CommunityProfilePatch } from "../../../../src/ipc/commands/dto.ts";

type Colour = readonly [number, number, number];
const RED = [215, 48, 67] as const;
const BLUE = [48, 101, 218] as const;
const AVATAR_PREVIEW = 'img[alt="Avatar preview"]';
const BANNER_PREVIEW = 'img[alt="Banner preview"]';
const FORM = ".profile-editor-form";
const NEW_BIO = "A new bio, saved by this window";
const NEW_STATUS = "The Host changed this status while the editor was open";

interface OwnProfile {
  pseudonymKey: string;
  bio?: string | null;
  customStatus?: string | null;
  avatarRef?: string | null;
  bannerRef?: string | null;
}

async function choose(button: string, name: string, colour: Colour): Promise<void> {
  const control = await shown(browser, button, NET_WAIT_MS);
  await control.waitForEnabled({ timeout: NET_WAIT_MS });
  const data = new Uint8Array(12 * 12 * 4);
  for (let i = 0; i < data.length; i += 4) data.set([...colour, 255], i);
  await pickNextFile(browser, name, "image/png", encodePng({ width: 12, height: 12, data }).toString("base64"));
  await press(browser, control);
}

/** Read the painted centre from the driver's own screenshot. The centre is
 * inside both the rounded avatars and the fitted banner. */
async function painted(selector: string, colour: Colour, name: string): Promise<void> {
  await shown(browser, selector, NET_WAIT_MS);
  const expected = [...colour, 255];
  let sampled = "no loaded image in the viewport";
  try {
    await waitUntil(async () => {
      const point = await browser.execute((s: string) => {
        const img = document.querySelector<HTMLImageElement>(s);
        if (!img?.complete || !img.naturalWidth) return null;
        img.scrollIntoView({ block: "nearest", inline: "nearest" });
        const r = img.getBoundingClientRect();
        const x = r.left + r.width / 2;
        const y = r.top + r.height / 2;
        if (x <= 2 || y <= 2 || x >= innerWidth - 2 || y >= innerHeight - 2) return null;
        return { x, y, width: innerWidth };
      }, selector);
      if (!point) return false;
      const image = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
      const scale = image.width / point.width;
      const x = Math.floor(point.x * scale);
      const y = Math.floor(point.y * scale);
      const data = new Uint8Array(3 * 3 * 4);
      for (let row = 0; row < 3; row++) {
        const first = ((y - 1 + row) * image.width + x - 1) * 4;
        data.set(image.data.subarray(first, first + 12), row * 12);
      }
      sampled = Array.from(data.subarray(16, 20)).join(", ");
      // The Host can re-encode as WebP, and the driver can colour-convert.
      const opaqueFixture = data.every((value, i) => Math.abs(value - expected[i % 4]) <= 12);
      if (!opaqueFixture) return false;
      saveImage(`${name}-pixels`, { width: 3, height: 3, data });
      return true;
    }, NET_WAIT_MS, `${name} never painted the fixture`);
  } catch {
    throw new Error(`${name} expected RGB ${colour.join(", ")}, sampled ${sampled}`);
  }
  await shot(name);
}

async function readOwnProfile(communityId: string, pseudonym: string): Promise<OwnProfile | undefined> {
  return browser.execute(async (id: string, me: string) => {
    const { invoke } = (window as unknown as {
      __TAURI_INTERNALS__: { invoke(cmd: string, args: object): Promise<{ value: OwnProfile[] }> };
    }).__TAURI_INTERNALS__;
    const { value } = await invoke("get_community_members", { communityId: id });
    return value.find((row) => row.pseudonymKey === me);
  }, communityId, pseudonym);
}

async function publishStatus(communityId: string, status: string): Promise<void> {
  // Submit an independent write through the real Host command. It emits
  // memberCardChanged, which updates the card shown behind our open form.
  await browser.execute(async (id: string, status: string) => {
    const { invoke } = (window as unknown as {
      __TAURI_INTERNALS__: { invoke(cmd: string, args: object): Promise<unknown> };
    }).__TAURI_INTERNALS__;
    const unchanged = { op: "unchanged" } as const;
    const patch: CommunityProfilePatch = {
      bio: unchanged,
      pronouns: unchanged,
      themeColor: unchanged,
      badges: unchanged,
      avatarRef: unchanged,
      bannerRef: unchanged,
      customStatus: { op: "set", value: status },
      displayName: unchanged,
    };
    await invoke("update_community_profile", { communityId: id, patch });
  }, communityId, status);
}

async function openEdit(pseudonym: string): Promise<void> {
  await press(browser, await shown(browser, `.member-item[data-pseudonym="${pseudonym}"]`, NET_WAIT_MS));
  await press(browser, await byText(browser, ".profile-popup-action-btn", "Edit profile"));
  await shown(browser, FORM);
}

async function editField(label: string): Promise<Awaited<ReturnType<typeof browser.$>>> {
  return (await byText(browser, ".profile-editor-label", label)).$("input, textarea");
}

describe("per-community profile survives a reopened window (PF2)", function () {
  this.timeout(2_400_000);
  const me = newUser("profile-reopen");
  const community = "Profile Hall";

  async function saveProfile(): Promise<void> {
    await press(browser, await shown(browser, `${FORM} button[type=submit]`));
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 3000 });
    await gone(browser, FORM, NET_WAIT_MS);
  }

  async function reopenProfile(pseudonym: string): Promise<void> {
    await browser.closeWindow();
    await openCommunity(browser, community);
    await selectChannel(browser, "general");
    await openEdit(pseudonym);
  }

  it("reloads both painted pictures, keeps a cleared banner absent, and preserves a newer Host field", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, community);
    await openCommunity(browser, community);
    await selectChannel(browser, "general");
    const communityId = await browser.execute(async (name: string) => {
      const { invoke } = (window as unknown as {
        __TAURI_INTERNALS__: { invoke(cmd: string): Promise<{ value: { anchor: string; name: string }[] }> };
      }).__TAURI_INTERNALS__;
      return (await invoke("get_communities")).value.find((row) => row.name === name)?.anchor;
    }, community);
    if (!communityId) throw new Error("the Host never listed the community");

    const pseudonym = await browser.execute(async (id: string) => {
      const { invoke } = (window as unknown as {
        __TAURI_INTERNALS__: { invoke(cmd: string): Promise<{ value: { id: string; myPseudonymKey: string | null }[] }> };
      }).__TAURI_INTERNALS__;
      return (await invoke("get_community_details")).value.find((row) => row.id === id)?.myPseudonymKey;
    }, communityId);
    if (!pseudonym) throw new Error("the Host did not return our community pseudonym");
    await openEdit(pseudonym);
    await choose('button[aria-label="Choose avatar image"]', "pf2-avatar-red.png", RED);
    await shown(browser, AVATAR_PREVIEW, NET_WAIT_MS);
    await choose('button[aria-label="Choose banner image"]', "pf2-banner-blue.png", BLUE);
    await shown(browser, BANNER_PREVIEW, NET_WAIT_MS);
    await saveProfile();
    const saved = await readOwnProfile(communityId, pseudonym);
    expect(saved?.avatarRef).toMatch(/^[0-9a-f]{64}$/);
    expect(saved?.bannerRef).toMatch(/^[0-9a-f]{64}$/);

    await reopenProfile(pseudonym);
    await painted(AVATAR_PREVIEW, RED, "pf2-reopened-avatar");
    await painted(BANNER_PREVIEW, BLUE, "pf2-reopened-banner");
    await press(browser, await shown(browser, 'button[aria-label="Remove banner"]'));
    await byText(browser, ".profile-editor-image-placeholder", "No banner");
    await gone(browser, BANNER_PREVIEW);
    await saveProfile();
    const cleared = await readOwnProfile(communityId, pseudonym);
    expect(cleared?.bannerRef ?? null).toBeNull();
    expect(cleared?.avatarRef).toBe(saved?.avatarRef);

    await reopenProfile(pseudonym);
    await painted(AVATAR_PREVIEW, RED, "pf2-avatar-after-clearing-banner");
    await byText(browser, ".profile-editor-image-placeholder", "No banner");
    await gone(browser, BANNER_PREVIEW);
    await gone(browser, 'button[aria-label="Remove banner"]');
    await shot("pf2-banner-still-absent");

    const before = await (await editField("Custom status")).getValue();
    await typeInto(await editField("Bio"), NEW_BIO);
    await publishStatus(communityId, NEW_STATUS);
    await byText(browser, ".member-item .member-custom-status", NEW_STATUS, NET_WAIT_MS);
    expect(await (await editField("Custom status")).getValue()).toBe(before);
    await saveProfile();
    const merged = await readOwnProfile(communityId, pseudonym);
    expect(merged?.bio).toBe(NEW_BIO);
    expect(merged?.customStatus).toBe(NEW_STATUS);
    expect(merged?.avatarRef).toBe(saved?.avatarRef);
    expect(merged?.bannerRef ?? null).toBeNull();
    await byText(browser, ".member-item .member-custom-status", NEW_STATUS, NET_WAIT_MS);
    await shot("pf2-new-bio-and-host-status");
  });
});
