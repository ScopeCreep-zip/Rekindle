import type { Page } from "@playwright/test";

/**
 * The Tauri commands that are Host calls (`src-tauri/src/shell_commands.rs`
 * `HOST_CALLS`): they answer `Done<T>` (`{ value, undo }`), which the mocks
 * build from a handler's plain value.
 */
const HOST_CALLS = [
  "create_identity", "login", "get_identity", "pqxdh_bundle_info", "logout", "list_identities",
  "delete_identity", "audit_verify", "audit_export", "lifecycle_current",
  "intent_commit", "intent_cancel", "undo", "set_status", "set_status_message",
  "set_nickname", "set_avatar", "get_avatar", "get_game_status", "open_external_url",
  // Friends and the 1:1 paths (plan E2.1).
  "send_message", "outbox_retry", "outbox_discard", "send_typing", "get_message_history", "mark_read", "add_friend_from_code",
  "get_friend_code", "rotate_friend_token",
  "remove_friend", "accept_request", "reject_request", "cancel_request", "retry_handshake", "get_friends",
  "get_pending_requests", "create_friend_group", "rename_friend_group",
  "delete_friend_group", "move_friend_to_group", "set_friend_nickname", "generate_invite",
  "add_friend_from_invite", "cancel_invite", "get_outgoing_invites", "block_user",
  "unblock_user", "get_blocked_users", "reset_signal_session",
  // Safety numbers and verification (plan E2.3).
  "get_safety_number", "mark_identity_verified", "clear_identity_verification",
];

/**
 * Set up Tauri IPC mocks in the browser page context.
 * Must be called after page.goto() since the mock functions are
 * injected by index.html when VITE_PLAYWRIGHT=true.
 *
 * `ipcHandler` is JS *source* (not a function) because it is compiled
 * with `new Function` inside the page — closures cannot cross the
 * page.evaluate boundary. To vary a few commands on top of a base
 * handler, pass `overrides`: a plain command → response map, consulted
 * before the base handler. Values must be JSON-serializable for the
 * same reason; a thunk would not survive the crossing.
 */
export async function setupMocks(
  page: Page,
  windowLabel: string,
  ipcHandler: string,
  overrides?: Record<string, unknown>,
) {
  // index.html installs the mock hooks behind an async import, so they
  // may not exist yet when `goto` resolves.
  await page.waitForFunction(() => typeof (window as any).__mockIPC === "function");
  await page.evaluate(
    ({ label, handler, cmdOverrides, hostCalls }) => {
      // Mock window labels so getCurrent() works
      (window as any).__mockWindows(label, "buddy-list", "login");

      // Mock IPC with the provided handler function
      const handlerFn = new Function("cmd", "args", handler);
      (window as any).__mockIPC(
        (cmd: string, args: Record<string, unknown>) => {
          const value =
            cmdOverrides && Object.prototype.hasOwnProperty.call(cmdOverrides, cmd)
              ? cmdOverrides[cmd]
              : handlerFn(cmd, args);
          return hostCalls.includes(cmd) ? { value, undo: null } : value;
        },
      );
    },
    {
      label: windowLabel,
      handler: ipcHandler,
      cmdOverrides: overrides ?? null,
      hostCalls: HOST_CALLS,
    },
  );
}

/**
 * Install IPC mocks before any app script runs, for windows that call IPC
 * during their first mount (the login window reads identities and the
 * lifecycle state, the buddy list hydrates). Installs the same
 * `__TAURI_INTERNALS__` shim as `mockIPC` + `mockWindows` from
 * `@tauri-apps/api/mocks`. Call before `page.goto`.
 *
 * Every call is recorded in `window.__ipcCalls` as `{ cmd, args }`.
 * `lifecycle_current` answers `"locked"` (node attached, no identity
 * unlocked), modelling readiness the same way the E2E server does, so the
 * login button is enabled.
 */
export async function preloadMocks(page: Page, windowLabel: string, ipcHandler: string) {
  await page.addInitScript(
    ({ label, handler, hostCalls }) => {
      const w = window as any;
      const handlerFn = new Function("cmd", "args", handler);
      const callbacks = new Map<number, (data: unknown) => void>();
      w.__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: { label },
          currentWebview: { windowLabel: label, label },
        },
        invoke: async (cmd: string, args: Record<string, unknown>) => {
          (w.__ipcCalls ??= []).push({ cmd, args });
          const value = cmd === "lifecycle_current" ? "locked" : await handlerFn(cmd, args);
          return hostCalls.includes(cmd) ? { value, undo: null } : value;
        },
        transformCallback: (cb: (data: unknown) => void) => {
          const id = window.crypto.getRandomValues(new Uint32Array(1))[0];
          callbacks.set(id, cb);
          return id;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        runCallback: (id: number, data: unknown) => callbacks.get(id)?.(data),
        callbacks,
      };
      w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: (_event: string, id: number) => callbacks.delete(id),
      };
    },
    { label: windowLabel, handler: ipcHandler, hostCalls: HOST_CALLS },
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
      return [{ publicKey: "abc123def456", displayName: "TestUser", createdAt: 1000 }];
    case "delete_identity":
      // Protected: the Host holds it for the identity's passphrase.
      throw {
        type: "intentRequired",
        data: {
          intentId: "01900000-0000-7000-8000-000000000001",
          kind: "deleteIdentity",
          class: "protected",
          subject: { ownIdentity: { name: "TestUser" } },
          facts: [],
          escalations: [],
          reversibility: "never",
          notifies: "nobody",
          proof: "passphrase",
          expiresAtMs: Date.now() + 60000,
          cancelSends: false,
        },
      };
    case "intent_commit":
      return { deleted: args.intentId };
    case "intent_cancel":
      // The error the cancelled call ends with (CommandError::Cancelled).
      return { type: "cancelled" };
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
        readReceiptsEnabled: false,
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
      return [{ publicKey: "abc123def456", displayName: "TestUser", createdAt: 1000 }];
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
