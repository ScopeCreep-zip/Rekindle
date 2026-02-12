import type { Page } from "@playwright/test";

/**
 * Set up Tauri IPC mocks in the browser page context.
 * Must be called after page.goto() since the mock functions are
 * injected by index.html when VITE_PLAYWRIGHT=true.
 */
export async function setupMocks(
  page: Page,
  windowLabel: string,
  ipcHandler: string,
) {
  await page.evaluate(
    ({ label, handler }) => {
      // Mock window labels so getCurrent() works
      (window as any).__mockWindows(label, "buddy-list", "login");

      // Mock IPC with the provided handler function
      const handlerFn = new Function("cmd", "args", handler);
      (window as any).__mockIPC(
        (cmd: string, args: Record<string, unknown>) => {
          return handlerFn(cmd, args);
        },
      );
    },
    { label: windowLabel, handler: ipcHandler },
  );
}

export async function clearMocks(page: Page) {
  await page.evaluate(() => {
    if ((window as any).__clearMocks) {
      (window as any).__clearMocks();
    }
  });
}

/**
 * Mock handler with one existing identity — picker shows, then login succeeds.
 */
export const LOGIN_SUCCESS_HANDLER = `
  switch (cmd) {
    case "list_identities":
      return [{ publicKey: "abc123def456", displayName: "TestUser", createdAt: 1000, hasAvatar: false, avatarBase64: null }];
    case "login":
      return { publicKey: "abc123def456", displayName: "TestUser" };
    case "create_identity":
      return { publicKey: "abc123def456", displayName: args.displayName || "TestUser" };
    case "show_buddy_list":
      return null;
    case "get_identity":
      return null;
    case "get_preferences":
      return {
        notificationsEnabled: true,
        notificationSound: true,
        startMinimized: false,
        autoStart: false,
        gameDetectionEnabled: true,
        gameScanIntervalSecs: 30,
      };
    default:
      return null;
  }
`;

/**
 * Mock handler with one existing identity — login throws wrong passphrase.
 */
export const LOGIN_FAIL_HANDLER = `
  switch (cmd) {
    case "list_identities":
      return [{ publicKey: "abc123def456", displayName: "TestUser", createdAt: 1000, hasAvatar: false, avatarBase64: null }];
    case "login":
      throw new Error("Wrong passphrase \\u2014 unable to unlock keystore");
    case "create_identity":
      throw new Error("Identity creation failed");
    case "show_buddy_list":
      return null;
    case "get_identity":
      return null;
    default:
      return null;
  }
`;

/**
 * Mock handler with NO identities — fresh install, goes directly to create mode.
 */
export const LOGIN_NO_IDENTITY_HANDLER = `
  switch (cmd) {
    case "list_identities":
      return [];
    case "login":
      throw new Error("no identity found \\u2014 please create one first");
    case "create_identity":
      return { publicKey: "new-key-789", displayName: args.displayName || "NewUser" };
    case "show_buddy_list":
      return null;
    case "get_identity":
      return null;
    default:
      return null;
  }
`;
