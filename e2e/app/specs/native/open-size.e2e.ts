// Plan E7.14 and WL2 (2026-11-20): every window opens at the size the shell
// builds it at (`src-tauri/src/windows/mod.rs` `size()`, the `def` of each
// case in windows.ts), on the session's own display (wdio.native.conf.ts).
// This is the size a user gets: on Wayland a client cannot resize itself
// (xdg-shell), so the opening size, and what the compositor grants on
// maximize, are the only sizes there are until the user drags.
//
// It guards WL2: on GTK Wayland every undecorated window opened taller than
// built, by the titlebar height GTK allocated before tao turned decorations
// off and set an empty CSD titlebar (tao issue #929; tao 0.37.1
// platform_impl/linux/window.rs:110 then 192-196; COSMIC measured the login
// window at 380×517 for 380×480). WL2 is fixed (round 8): on native Wayland
// at ddfc1e70b every window opens at its built size, so every display,
// Wayland included, must give each window its built size exactly.
//
// On native Wayland each window is also maximized (xdg_toplevel
// set_maximized: the compositor's size is binding) and checked there. Such
// a window is on screen, where the real pointer may hover it, so the checks
// of what is rendered are left out (geometry.ts `onScreen`). The call window
// is not opened: it asks for the camera and microphone.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, shown, switchTo, waitForHandle } from "../../../walk/helpers.ts";
import { BUDDY, LOGIN as LOGIN_LABEL, SETTINGS, openSettings } from "../../../walk/nav.ts";
import { type Peer, startFleet, stopFleet } from "../../fleet.ts";
import { checkGeometry } from "../../geometry.ts";
import { createCommunity, createIdentity, friendRow, newUser, openChat, openCommunity, type User } from "../../journeys.ts";
import { befriendFleet } from "../../social.ts";
import { BUDDY_LIST, CHAT_DM, COMMUNITY, LOGIN, PROFILE, SETTINGS_PROFILE, SKIN_EDITOR, type Size, type WindowCase } from "../../windows.ts";

/** A Wayland session whose app runs as a Wayland client (this config's `display: "native"`). */
const NATIVE_WAYLAND = process.platform === "linux" && Boolean(process.env.WAYLAND_DISPLAY) && process.env.GDK_BACKEND !== "x11";

/** The window `browser` is in opened at `w`'s built size; on native Wayland, maximized, it passes the structure checks. */
async function opensAsBuilt(w: WindowCase): Promise<void> {
  const viewport = (await browser.execute(() => ({ width: window.innerWidth, height: window.innerHeight }))) as Size;
  expect(viewport).toEqual(w.def);
  if (NATIVE_WAYLAND) {
    await browser.maximizeWindow();
    const found = await checkGeometry(browser, ".app-frame", { spacing: w.spacing, panes: w.panes ?? [], overlapAllow: w.overlapAllow, onScreen: true });
    expect(found).toEqual({});
  }
}

describe("every window opens at the size the shell builds it at", function () {
  this.timeout(600_000);
  const me: User = newUser("opensize");
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("login window, on first run", async () => {
    await switchTo(browser, LOGIN_LABEL);
    await opensAsBuilt(LOGIN);
  });

  it("buddy list", async () => {
    await createIdentity(browser, me);
    await switchTo(browser, BUDDY);
    await opensAsBuilt(BUDDY_LIST);
  });

  it("settings", async () => {
    await openSettings(browser, "Profile");
    await opensAsBuilt(SETTINGS_PROFILE);
  });

  it("skin editor", async () => {
    await switchTo(browser, SETTINGS);
    await (await byText(browser, ".form-tab", "Appearance")).click();
    const before = await browser.getWindowHandles();
    await (await byText(browser, "button.form-btn-secondary", "Edit a skin")).click();
    await browser.switchToWindow(await waitForHandle(browser, (h) => h === "skin-editor" && !before.includes(h), NET_WAIT_MS));
    await shown(browser, ".skin-pair", NET_WAIT_MS);
    await opensAsBuilt(SKIN_EDITOR);
  });

  it("community window", async () => {
    await createCommunity(browser, me, "Open Size");
    await openCommunity(browser, "Open Size");
    await opensAsBuilt(COMMUNITY);
  });

  it("1:1 chat window", async () => {
    peers = await startFleet(["PeerOne"]);
    await befriendFleet(browser, me, peers);
    await openChat(browser, peers[0].displayName);
    await opensAsBuilt(CHAT_DM);
  });

  it("a friend's profile window", async () => {
    const row = await friendRow(browser, peers[0].displayName);
    await browser.execute((node: HTMLElement) => {
      const r = node.getBoundingClientRect();
      node.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: r.x + 5, clientY: r.y + 5 }));
    }, row as unknown as HTMLElement);
    await press(browser, await byText(browser, ".context-menu-item", "View Profile"));
    await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("profile-")));
    await shown(browser, ".profile-actions");
    await opensAsBuilt(PROFILE);
  });
});
