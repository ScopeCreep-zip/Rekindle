import { test, expect } from "@playwright/test";
import {
  preloadMocks,
  clearMocks,
  LOGIN_SUCCESS_HANDLER,
  LOGIN_FAIL_HANDLER,
  LOGIN_NO_IDENTITY_HANDLER,
} from "./fixtures/mocks";

// Mocks are installed before the page loads (`preloadMocks`): the login
// window reads identities and the lifecycle state while it mounts.

/// The login/create form's submit button (`LoadingButton`).
const SUBMIT = '.login-container button[type="submit"]';
/// "← Switch Account" (login mode) or "← Back" (create mode).
const BACK = /Switch Account|Back/;

const ONE_IDENTITY = `[{ publicKey: "abc123def456", displayName: "TestUser", createdAt: 1000, hasAvatar: false, avatarBase64: null }]`;

const TRACKED_LOGIN_HANDLER = `
  switch (cmd) {
    case "list_identities": return ${ONE_IDENTITY};
    case "login": return { publicKey: "abc123def456", displayName: "TestUser" };
    default: return null;
  }
`;

const EMPTY_PASSPHRASE_HANDLER = `
  return cmd === "list_identities" ? ${ONE_IDENTITY} : null;
`;

const SLOW_LOGIN_HANDLER = `
  switch (cmd) {
    case "list_identities": return ${ONE_IDENTITY};
    case "login":
      return new Promise((resolve) =>
        setTimeout(() => resolve({ publicKey: "abc123def456", displayName: "TestUser" }), 2000),
      );
    default: return null;
  }
`;

const CREATE_FRESH_HANDLER = `
  switch (cmd) {
    case "list_identities": return [];
    case "create_identity": return { publicKey: "new-key-789", displayName: args.displayName || "Anonymous" };
    default: return null;
  }
`;

const CREATE_FROM_PICKER_HANDLER = `
  switch (cmd) {
    case "list_identities":
      return [{ publicKey: "abc123def456", displayName: "ExistingUser", createdAt: 1000, hasAvatar: false, avatarBase64: null }];
    case "create_identity": return { publicKey: "new-key-789", displayName: args.displayName || "Anonymous" };
    default: return null;
  }
`;

// ── Account Picker ──────────────────────────────────────────────────

test.describe("Account Picker", () => {
  test.afterEach(async ({ page }) => {
    await clearMocks(page);
  });

  test("shows picker with existing identity", async ({ page }) => {
    await preloadMocks(page, "login", LOGIN_SUCCESS_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Title
    await expect(page.locator(".login-title")).toHaveText("Rekindle");

    // Subtitle — picker mode
    await expect(page.locator(".account-picker-subtitle")).toContainText(
      "Select an account",
    );

    // Account card with TestUser
    await expect(page.locator(".account-bubble-name")).toHaveText("TestUser");

    // Create new identity button
    await expect(page.locator(".account-create-btn")).toContainText(
      "Create New Identity",
    );
  });

  test("clicking account card transitions to login mode", async ({
    page,
  }) => {
    await preloadMocks(page, "login", LOGIN_SUCCESS_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Click the account card
    await page.locator(".account-bubble").click();

    // Should show lock screen with selected user's name
    await expect(page.locator(".lock-name")).toContainText("TestUser");

    // Passphrase input visible
    const passphraseInput = page.getByPlaceholder("Passphrase");
    await expect(passphraseInput).toBeVisible();

    // Unlock button
    await expect(page.locator(SUBMIT)).toHaveText("Unlock");

    // Back button visible
    await expect(page.getByRole("button", { name: BACK })).toBeVisible();
  });

  test("clicking Create New Identity transitions to create mode", async ({
    page,
  }) => {
    await preloadMocks(page, "login", LOGIN_SUCCESS_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Click create button
    await page.locator(".account-create-btn").click();

    // Should show create mode
    await expect(page.locator(".login-subtitle")).toContainText(
      "Create a passphrase",
    );

    // Display name input appears
    const displayNameInput = page.locator('input[type="text"]');
    await expect(displayNameInput).toBeVisible();
    await expect(displayNameInput).toHaveAttribute(
      "placeholder",
      "Display Name (optional)",
    );

    // Button text
    await expect(page.locator(SUBMIT)).toHaveText("Create Identity");

    // Back button visible (since identities exist)
    await expect(page.getByRole("button", { name: BACK })).toBeVisible();
  });

  test("back button returns to picker from login mode", async ({ page }) => {
    await preloadMocks(page, "login", LOGIN_SUCCESS_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Go to login mode
    await page.locator(".account-bubble").click();
    await expect(page.locator(".lock-name")).toBeVisible();

    // Go back
    await page.getByRole("button", { name: BACK }).click();

    // Should be back at picker
    await expect(page.locator(".account-picker-subtitle")).toContainText(
      "Select an account",
    );
  });
});

// ── Login Flow ──────────────────────────────────────────────────────

test.describe("Login Flow", () => {
  test.afterEach(async ({ page }) => {
    await clearMocks(page);
  });

  test("successful login via picker calls login and show_buddy_list", async ({
    page,
  }) => {
    await preloadMocks(page, "login", TRACKED_LOGIN_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Click account card → login mode
    await page.locator(".account-bubble").click();

    // Type passphrase and submit
    await page.locator('input[type="password"]').fill("my-secret-passphrase");
    await page.locator(SUBMIT).click();

    // Wait for the IPC calls to complete
    await page.waitForFunction(
      () => {
        const cmds = ((window as any).__ipcCalls ?? []).map((c: { cmd: string }) => c.cmd);
        return cmds.includes("login") && cmds.includes("show_buddy_list");
      },
      null,
      { timeout: 5000 },
    );

    // Verify the commands were called in order
    const calls: string[] = await page.evaluate(() =>
      (window as any).__ipcCalls.map((c: { cmd: string }) => c.cmd),
    );
    expect(calls).toContain("login");
    expect(calls).toContain("show_buddy_list");
    expect(calls.indexOf("login")).toBeLessThan(
      calls.indexOf("show_buddy_list"),
    );
  });

  test("failed login shows error message", async ({ page }) => {
    await preloadMocks(page, "login", LOGIN_FAIL_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Navigate through picker to login mode
    await page.locator(".account-bubble").click();

    // Type passphrase and submit
    await page.locator('input[type="password"]').fill("wrong-passphrase");
    await page.locator(SUBMIT).click();

    // Error message should appear
    await expect(page.locator(".form-error")).toBeVisible({ timeout: 5000 });
    await expect(page.locator(".form-error")).toContainText(
      "Wrong passphrase",
    );

    // Button should return to normal (not loading)
    await expect(page.locator(SUBMIT)).toHaveText("Unlock");
    await expect(page.locator(SUBMIT)).not.toBeDisabled();
  });

  test("empty passphrase does not submit", async ({ page }) => {
    await preloadMocks(page, "login", EMPTY_PASSPHRASE_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Navigate to login mode
    await page.locator(".account-bubble").click();

    // Click submit with empty passphrase
    await page.locator(SUBMIT).click();

    // Small delay to ensure nothing fires
    await page.waitForTimeout(500);

    // No login IPC call should have been made
    const calls: string[] = await page.evaluate(() =>
      (window as any).__ipcCalls.map((c: { cmd: string }) => c.cmd),
    );
    expect(calls).not.toContain("login");
  });

  test("loading state disables button during login", async ({ page }) => {
    await preloadMocks(page, "login", SLOW_LOGIN_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Navigate to login mode
    await page.locator(".account-bubble").click();

    await page.locator('input[type="password"]').fill("test-passphrase");
    await page.locator(SUBMIT).click();

    // Button should show loading state
    await expect(page.locator(SUBMIT)).toContainText("Unlocking");
    await expect(page.locator(SUBMIT)).toBeDisabled();
  });
});

// ── Create Identity Flow ────────────────────────────────────────────

test.describe("Create Identity Flow", () => {
  test.afterEach(async ({ page }) => {
    await clearMocks(page);
  });

  test("fresh install shows create mode directly (no picker)", async ({
    page,
  }) => {
    await preloadMocks(page, "login", LOGIN_NO_IDENTITY_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Should be in create mode — no picker visible
    await expect(page.locator(".account-picker")).not.toBeVisible();
    await expect(page.locator(".login-subtitle")).toContainText(
      "Create a passphrase",
    );

    // No back button when no identities exist
    await expect(page.getByRole("button", { name: BACK })).not.toBeVisible();

    // Create button
    await expect(page.locator(SUBMIT)).toHaveText("Create Identity");
  });

  test("create identity submits with display name and passphrase", async ({
    page,
  }) => {
    await preloadMocks(page, "login", CREATE_FRESH_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Should already be in create mode (no identities)
    await expect(page.locator(".login-subtitle")).toContainText(
      "Create a passphrase",
    );

    // Fill in display name and passphrase
    await page.locator('input[type="text"]').fill("CoolUser");
    await page.locator('input[type="password"]').fill("my-new-passphrase");
    await page.locator(SUBMIT).click();

    // Wait for IPC calls
    await page.waitForFunction(
      () =>
        (window as any).__ipcCalls?.some(
          (c: { cmd: string }) => c.cmd === "create_identity",
        ),
      null,
      { timeout: 5000 },
    );

    const calls: Array<{ cmd: string; args: Record<string, unknown> }> =
      await page.evaluate(() => (window as any).__ipcCalls);

    // Verify create_identity was called with the right args
    const createCall = calls.find((c) => c.cmd === "create_identity");
    expect(createCall).toBeDefined();
    expect(createCall!.args).toMatchObject({
      passphrase: "my-new-passphrase",
      displayName: "CoolUser",
    });
  });

  test("create from picker via Create New Identity button", async ({
    page,
  }) => {
    await preloadMocks(page, "login", CREATE_FROM_PICKER_HANDLER);
    await page.goto("/login");

    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Should be in picker mode
    await expect(page.locator(".account-picker-subtitle")).toContainText(
      "Select an account",
    );

    // Click "Create New Identity"
    await page.locator(".account-create-btn").click();

    // Should be in create mode with back button
    await expect(page.locator(".login-subtitle")).toContainText(
      "Create a passphrase",
    );
    await expect(page.getByRole("button", { name: BACK })).toBeVisible();

    // Fill and submit
    await page.locator('input[type="text"]').fill("SecondUser");
    await page.locator('input[type="password"]').fill("second-pass");
    await page.locator(SUBMIT).click();

    // Wait for create
    await page.waitForFunction(
      () =>
        (window as any).__ipcCalls?.some(
          (c: { cmd: string }) => c.cmd === "create_identity",
        ),
      null,
      { timeout: 5000 },
    );

    const calls: Array<{ cmd: string; args: Record<string, unknown> }> =
      await page.evaluate(() => (window as any).__ipcCalls);

    const createCall = calls.find((c) => c.cmd === "create_identity");
    expect(createCall).toBeDefined();
    expect(createCall!.args).toMatchObject({
      passphrase: "second-pass",
      displayName: "SecondUser",
    });
  });
});
