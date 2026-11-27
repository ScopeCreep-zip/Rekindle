// Contract PN1: the role editor lists the Host's permission catalogue
// (get_permission_catalogue), never a shell copy. Ari opens Create Role in
// her community: every group and label the Host serves is shown, in its
// order and nothing else; she ticks one permission and creates the role, and
// the Host's role carries exactly that bit.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, resolveIntent, shown, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { openCommunitySettings } from "../../../walk/rows-community.ts";
import { createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Role Hall ${Date.now() % 100000}`;
const ROLE = "catalogue-role";
/** The permission ticked: any key the catalogue serves will do; this one is plain. */
const TICKED = "ADD_REACTIONS";

interface Catalogue {
  groups: { name: string; permissions: { key: string; label: string; bit: string }[] }[];
}

/** `cmd` through the window's raw invoke. Its args are always an object:
 *  WebDriver serialises an absent argument as null, and the isolation hook
 *  refuses a payload that is not a plain object by throwing, which leaves the
 *  invoke unanswered (dist-isolation/index.js). */
async function hostCall<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return (await browser.execute(async (c: string, a: Record<string, unknown> | undefined) => {
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args?: unknown): Promise<{ value: unknown }> } }).__TAURI_INTERNALS__;
    return (await invoke(c, a)).value;
  }, cmd, args)) as T;
}

describe("the role editor lists the Host's permission catalogue (PN1)", function () {
  this.timeout(600_000);
  const me: User = { ...newUser("roles"), displayName: "Ari" };

  it("shows every group and label the Host serves, and a ticked permission is saved on the role", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, COMMUNITY);
    await openCommunity(browser, COMMUNITY);
    const catalogue = await hostCall<Catalogue>("get_permission_catalogue");
    await openCommunitySettings(browser, "Roles");
    await press(browser, await byText(browser, ".modal-container .form-btn-secondary", "Create Role"));
    await typeInto(await shown(browser, 'input[placeholder="Role name..."]'), ROLE);
    await shown(browser, `label.settings-option[data-permission="${TICKED}"]`, NET_WAIT_MS);

    // What the editor shows, in order: each group's name and its labels by key.
    const shownGroups = (await browser.execute(() =>
      Array.from(document.querySelectorAll(".modal-container .settings-category-label")).map((g) => ({
        name: (g.textContent ?? "").trim(),
        permissions: Array.from(g.parentElement?.querySelectorAll("label.settings-option[data-permission]") ?? []).map((l) => ({
          key: l.getAttribute("data-permission") ?? "",
          label: (l.textContent ?? "").trim(),
        })),
      })),
    )) as { name: string; permissions: { key: string; label: string }[] }[];
    const served = catalogue.groups.map((g) => ({ name: g.name, permissions: g.permissions.map((p) => ({ key: p.key, label: p.label })) }));
    console.log(`[roles] the Host serves ${served.length} groups, ${served.reduce((n, g) => n + g.permissions.length, 0)} permissions`);
    await shot("role-editor-catalogue");

    // Tick one and create the role.
    await press(browser, await shown(browser, `label.settings-option[data-permission="${TICKED}"] input`));
    await press(browser, await byText(browser, ".modal-container .form-btn-primary", "Create"));
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 3000 });
    await byText(browser, ".settings-role-row", ROLE, NET_WAIT_MS);

    // The Host's role carries exactly the ticked bit.
    const bit = BigInt(catalogue.groups.flatMap((g) => g.permissions).find((p) => p.key === TICKED)!.bit);
    let saved: bigint | null = null;
    await waitUntil(
      async () => {
        const details = await hostCall<{ name: string; roles: { name: string; permissions: string | number }[] }[]>("get_community_details");
        const role = details.find((d) => d.name === COMMUNITY)?.roles.find((r) => r.name === ROLE);
        saved = role ? BigInt(role.permissions) : null;
        return saved !== null;
      },
      NET_WAIT_MS,
      `the Host never listed the role ${ROLE}`,
    );
    console.log(`[roles] saved permissions ${String(saved)} (ticked bit ${String(bit)})`);
    await shot("role-created");

    expect({ editor: shownGroups, saved: saved === bit }).toEqual({ editor: served, saved: true });
  });
});
