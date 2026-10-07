import { test, expect } from "@playwright/test";
import { preloadMocks, LOGIN_SUCCESS_HANDLER } from "../fixtures/mocks";

// Deep-link consent.
//
// OS deep links (`rekindle://invite/{VLD0:…}/{VLD0:…}/{32 hex}`, friend
// invites `rekindle://<base64url blob>`, pairing `rekindle://pair?…`) are
// parsed and validated in Rust (`rekindle_types::invite::{InviteLink,
// DeepLink}`; the hostile-URL corpus is unit-tested there). The backend
// holds a valid link and hands the webview only a request id and a key
// fingerprint. These tests check the webview side of that contract:
//   * the buddy list pulls the pending request on mount and asks first;
//   * nothing is joined until the user confirms;
//   * Cancel dismisses the request in the backend;
//   * a refused pairing link only informs.

const JOIN_REQUEST = {
  kind: "joinCommunity",
  requestId: "0f1e2d3c4b5a69788796a5b4c3d2e1f0",
  keyFingerprint: "3f1a 9c0b 7e2d 4f6a",
};

/// The login handler, plus the given pending request.
function handlerWithPending(request: unknown): string {
  return `
    if (cmd === "get_pending_deep_link") return ${JSON.stringify(request)};
    if (cmd === "confirm_deep_link") return { kind: "dismissed" };
    ${LOGIN_SUCCESS_HANDLER}
  `;
}

async function ipcCalls(page: import("@playwright/test").Page): Promise<string[]> {
  return page.evaluate(() =>
    ((window as unknown as { __ipcCalls?: { cmd: string }[] }).__ipcCalls ?? []).map((c) => c.cmd),
  );
}

test.describe("Deep-link consent", () => {
  test("asks before joining and joins only on confirm", async ({ page }) => {
    await preloadMocks(page, "buddy-list", handlerWithPending(JOIN_REQUEST));
    await page.goto("/buddy-list");

    const dialog = page.getByRole("dialog", { name: "Join community?" });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(JOIN_REQUEST.keyFingerprint);
    expect(await ipcCalls(page)).not.toContain("confirm_deep_link");

    await dialog.getByRole("button", { name: "Join" }).click();
    await expect(dialog).toBeHidden();
    expect(await ipcCalls(page)).toContain("confirm_deep_link");
  });

  test("cancel dismisses the request", async ({ page }) => {
    await preloadMocks(page, "buddy-list", handlerWithPending(JOIN_REQUEST));
    await page.goto("/buddy-list");

    const dialog = page.getByRole("dialog", { name: "Join community?" });
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
    const calls = await ipcCalls(page);
    expect(calls).toContain("dismiss_deep_link");
    expect(calls).not.toContain("confirm_deep_link");
  });

  test("a refused pairing link only informs", async ({ page }) => {
    const pairing = { kind: "pairingRefused", requestId: JOIN_REQUEST.requestId };
    await preloadMocks(page, "buddy-list", handlerWithPending(pairing));
    await page.goto("/buddy-list");

    const dialog = page.getByRole("dialog", { name: "Pairing link blocked" });
    await expect(dialog).toContainText("Settings → Devices");
    await expect(dialog.getByRole("button", { name: "Join" })).toHaveCount(0);
    await dialog.getByRole("button", { name: "OK" }).click();
    expect(await ipcCalls(page)).toContain("dismiss_deep_link");
  });
});
