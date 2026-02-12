/**
 * Real end-to-end auth flow tests.
 *
 * These tests run the actual SolidJS frontend against the real Rust backend
 * (SQLite + Stronghold + Ed25519) via the E2E HTTP bridge server.
 *
 * What's real:
 * - SolidJS UI rendering, routing, state management
 * - Argon2id key derivation, Ed25519 keypair generation
 * - Stronghold encrypted key storage on disk
 * - SQLite identity persistence
 *
 * What's different from production:
 * - IPC transport is HTTP instead of Tauri native IPC
 * - Window management commands (show_buddy_list) trigger browser navigation
 * - Tauri event system (listen) is no-oped
 */

import { test, expect } from "@playwright/test";

const E2E_SERVER = "http://127.0.0.1:3001";

/** Reset the E2E server state between tests for isolation. */
async function resetServer(): Promise<void> {
  const res = await fetch(`${E2E_SERVER}/reset`, { method: "POST" });
  if (!res.ok) {
    throw new Error(`Failed to reset E2E server: ${res.status}`);
  }
}

/** Create an identity directly via the E2E server (for test setup). */
async function createIdentityViaServer(
  passphrase: string,
  displayName: string,
): Promise<{ publicKey: string; displayName: string }> {
  const res = await fetch(`${E2E_SERVER}/invoke`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      cmd: "create_identity",
      args: { passphrase, displayName },
    }),
  });
  const body = await res.json();
  if (!res.ok) throw new Error(body.error);
  return body.result;
}

/** Clear server identity state (simulate app restart). */
async function simulateRestart(): Promise<void> {
  await fetch(`${E2E_SERVER}/invoke`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ cmd: "logout", args: {} }),
  });
}

// ── New User Registration ───────────────────────────────────────────

test.describe("New User Registration", () => {
  test.beforeEach(async () => {
    await resetServer();
  });

  test("creates identity through the UI and lands on the buddy list", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Fresh install — should be in create mode directly (no picker)
    await expect(page.locator(".login-subtitle")).toContainText(
      "Create a passphrase",
    );

    // Fill in display name and passphrase
    await page.locator('input[type="text"]').fill("Alice");
    await page.locator('input[type="password"]').fill("my-secure-passphrase");

    // Submit — this calls real create_identity_core (Ed25519 + Stronghold + SQLite)
    await page.locator(".login-btn").click();

    // Should navigate to buddy list (show_buddy_list triggers browser nav in E2E)
    await page.waitForURL("**/buddy-list", { timeout: 15_000 });

    // Buddy list should hydrate from real backend and show "Alice"
    await expect(page.locator(".identity-bar-name")).toHaveText("Alice", {
      timeout: 10_000,
    });

    // Public key should be displayed (truncated)
    const keyText = await page.locator(".identity-bar-key").textContent();
    expect(keyText).toBeTruthy();
    expect(keyText!.length).toBeGreaterThan(10);
  });

  test("creates identity with default display name when left blank", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Fresh install — already in create mode
    // Leave display name empty, just fill passphrase
    await page.locator('input[type="password"]').fill("test-passphrase");
    await page.locator(".login-btn").click();

    // Should navigate to buddy list
    await page.waitForURL("**/buddy-list", { timeout: 15_000 });

    // Should have fallback name "User_" + 8 hex chars
    const displayName = await page
      .locator(".identity-bar-name")
      .textContent();
    expect(displayName).toMatch(/^User_[0-9a-f]{8}$/);
  });
});

// ── Existing User Login ─────────────────────────────────────────────

test.describe("Existing User Login", () => {
  const TEST_PASSPHRASE = "e2e-login-passphrase";
  let createdPublicKey: string;

  test.beforeAll(async () => {
    await resetServer();
    // Pre-create an identity via the server API
    const result = await createIdentityViaServer(TEST_PASSPHRASE, "Bob");
    createdPublicKey = result.publicKey;
    // Simulate app restart (clear in-memory state, keep DB + Stronghold)
    await simulateRestart();
  });

  test("logs in with correct passphrase via picker and reaches buddy list", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Should show the account picker with Bob's identity
    await expect(page.locator(".account-picker-subtitle")).toContainText(
      "Select an account",
    );
    await expect(page.locator(".account-bubble-name")).toHaveText("Bob");

    // Click Bob's account card
    await page.locator(".account-bubble").click();

    // Should transition to login mode
    await expect(page.locator(".login-subtitle")).toContainText(
      "Enter your passphrase to unlock",
    );

    // Enter the correct passphrase
    await page.locator('input[type="password"]').fill(TEST_PASSPHRASE);
    await page.locator(".login-btn").click();

    // Should navigate to buddy list
    await page.waitForURL("**/buddy-list", { timeout: 15_000 });

    // Should show "Bob" — the real display name from SQLite
    await expect(page.locator(".identity-bar-name")).toHaveText("Bob", {
      timeout: 10_000,
    });

    // Should show the same public key (proving Stronghold key roundtrip works)
    const keyText = await page.locator(".identity-bar-key").textContent();
    expect(keyText).toContain(createdPublicKey.slice(0, 8));
  });
});

// ── Login Failures ──────────────────────────────────────────────────

test.describe("Login Failures", () => {
  test.beforeAll(async () => {
    await resetServer();
    // Create an identity so Stronghold file exists
    await createIdentityViaServer("correct-password", "Charlie");
    await simulateRestart();
  });

  test("wrong passphrase shows error and stays on login page", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Click Charlie's card in the picker
    await page.locator(".account-bubble").click();

    // Enter wrong passphrase
    await page.locator('input[type="password"]').fill("wrong-password");
    await page.locator(".login-btn").click();

    // Should show the real Stronghold error (translated to user-friendly message)
    await expect(page.locator(".login-error")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(".login-error")).toContainText(
      "Wrong passphrase",
    );

    // Should still be on the login page (no navigation)
    expect(page.url()).toContain("/login");

    // Button should return to normal state
    await expect(page.locator(".login-btn")).toHaveText("Unlock");
    await expect(page.locator(".login-btn")).not.toBeDisabled();
  });

  test("empty passphrase does not submit", async ({ page }) => {
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Click Charlie's card
    await page.locator(".account-bubble").click();

    // Click submit without entering anything
    await page.locator(".login-btn").click();

    // Wait briefly to confirm nothing happens
    await page.waitForTimeout(500);

    // No error should appear (form just doesn't submit)
    await expect(page.locator(".login-error")).not.toBeVisible();

    // Still on login page
    expect(page.url()).toContain("/login");
  });
});

// ── No Identity (Fresh Install) ─────────────────────────────────────

test.describe("Fresh Install", () => {
  test.beforeEach(async () => {
    await resetServer();
  });

  test("fresh install shows create mode directly (no picker)", async ({
    page,
  }) => {
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    // Should be in create mode — no picker visible
    await expect(page.locator(".account-picker")).not.toBeVisible();
    await expect(page.locator(".login-subtitle")).toContainText(
      "Create a passphrase",
    );

    // No back button when no identities exist
    await expect(page.locator(".account-back-btn")).not.toBeVisible();
  });
});

// ── Full Lifecycle ──────────────────────────────────────────────────

test.describe("Full Lifecycle", () => {
  test.beforeEach(async () => {
    await resetServer();
  });

  test("create identity → logout → login via picker → same user restored", async ({
    page,
  }) => {
    // Step 1: Create identity (fresh install → create mode)
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });
    await page.locator('input[type="text"]').fill("Dave");
    await page.locator('input[type="password"]').fill("lifecycle-pass");
    await page.locator(".login-btn").click();

    // Step 2: Verify on buddy list with correct name
    await page.waitForURL("**/buddy-list", { timeout: 15_000 });
    await expect(page.locator(".identity-bar-name")).toHaveText("Dave", {
      timeout: 10_000,
    });

    // Capture the public key for later comparison
    const keyText = await page.locator(".identity-bar-key").textContent();

    // Step 3: Simulate app restart (logout clears in-memory state)
    await simulateRestart();

    // Step 4: Navigate back to login — picker should show Dave
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });
    await expect(page.locator(".account-bubble-name")).toHaveText("Dave");

    // Click Dave's card
    await page.locator(".account-bubble").click();

    // Enter passphrase
    await page.locator('input[type="password"]').fill("lifecycle-pass");
    await page.locator(".login-btn").click();

    // Step 5: Verify same identity restored
    await page.waitForURL("**/buddy-list", { timeout: 15_000 });
    await expect(page.locator(".identity-bar-name")).toHaveText("Dave", {
      timeout: 10_000,
    });

    // Same public key (proving Stronghold key roundtrip across "restarts")
    const keyText2 = await page.locator(".identity-bar-key").textContent();
    expect(keyText2).toBe(keyText);
  });
});

// ── Multi-Account ───────────────────────────────────────────────────

test.describe("Multi-Account", () => {
  test.beforeEach(async () => {
    await resetServer();
  });

  test("create two identities and switch between them via picker", async ({
    page,
  }) => {
    // Create identity A
    await createIdentityViaServer("pass-a", "Alice");
    await simulateRestart();

    // Create identity B
    await createIdentityViaServer("pass-b", "Bob");
    await simulateRestart();

    // Navigate to login — picker should show both
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    const cards = page.locator(".account-bubble");
    await expect(cards).toHaveCount(2);

    // Select Alice (first card)
    await cards.first().click();
    await expect(page.locator(".lock-name")).toContainText("Alice");

    // Enter Alice's passphrase
    await page.locator('input[type="password"]').fill("pass-a");
    await page.locator(".login-btn").click();

    // Should reach buddy list as Alice
    await page.waitForURL("**/buddy-list", { timeout: 15_000 });
    await expect(page.locator(".identity-bar-name")).toHaveText("Alice", {
      timeout: 10_000,
    });
  });

  test("delete an identity removes it from picker", async ({ page }) => {
    // Create identity
    await createIdentityViaServer("delete-me-pass", "DeleteMe");
    await simulateRestart();

    // Navigate to login — picker shows the identity
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });
    await expect(page.locator(".account-bubble")).toHaveCount(1);

    // Click delete button on the card
    await page.locator(".account-bubble-delete").click();

    // Delete confirmation modal should appear
    await expect(page.locator(".modal-container")).toBeVisible();
    await expect(page.locator(".delete-confirm-text")).toContainText("DeleteMe");

    // Enter passphrase and confirm deletion
    await page.locator(".modal-container input[type='password']").fill("delete-me-pass");
    await page.locator(".delete-confirm-btn").click();

    // Should transition to create mode (no identities left)
    await expect(page.locator(".login-subtitle")).toContainText(
      "Create a passphrase",
      { timeout: 10_000 },
    );

    // No account cards should be visible
    await expect(page.locator(".account-bubble")).not.toBeVisible();
  });
});
