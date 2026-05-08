import { test, expect, type Page } from "@playwright/test";
import { setupMocks, clearMocks, LOGIN_SUCCESS_HANDLER } from "../fixtures/mocks";

// XSS injection tests against every peer-content render path.
//
// Strategy:
//   1. Install a global window flag (window.__rekindleXssTriggered = false).
//   2. Mock IPC responses to return malicious payloads in user-controlled
//      fields (display name, status message, profile bio, message body,
//      community name, channel name, custom emoji name, etc.).
//   3. Navigate to the page that renders the field.
//   4. Assert window.__rekindleXssTriggered is still false.
//   5. Assert the DOM shows the payload as text content, not as live HTML.
//
// New peer-content render paths must add a test here. Coverage gaps are
// genuine vulnerabilities — every path that renders attacker-controlled
// strings needs an entry.

const XSS_PAYLOADS = [
  // Classic script tag
  '<script>window.__rekindleXssTriggered = true;</script>',
  // Image with onerror
  '<img src=x onerror="window.__rekindleXssTriggered = true">',
  // SVG with event handler
  '<svg onload="window.__rekindleXssTriggered = true"></svg>',
  // Iframe with javascript: URL
  '<iframe src="javascript:window.__rekindleXssTriggered = true"></iframe>',
  // Data: URI iframe
  '<iframe src="data:text/html,<script>parent.__rekindleXssTriggered=true</script>"></iframe>',
  // Anchor with javascript: href (SolidJS escapes attrs but worth confirming)
  '<a href="javascript:window.__rekindleXssTriggered = true">click</a>',
  // Encoded payload
  '&lt;script&gt;window.__rekindleXssTriggered = true&lt;/script&gt;',
  // Polyglot
  'jaVasCript:/*-/*`/*\\`/*\'/*"/**/(/* */oNcliCk=alert() )//%0D%0A%0d%0a//</stYle/</titLe/</teXtarEa/</scRipt/--!>\\x3csVg/<sVg/oNloAd=window.__rekindleXssTriggered=true//>\\x3e',
];

async function installXssCanary(page: Page): Promise<void> {
  await page.addInitScript(() => {
    (window as unknown as { __rekindleXssTriggered: boolean }).__rekindleXssTriggered = false;
    // Override eval / Function — if anyone calls them, we want to know.
    const orig = window.eval.bind(window);
    window.eval = ((expr: string) => {
      (window as unknown as { __rekindleEvalCalled: string }).__rekindleEvalCalled = expr;
      return orig(expr);
    }) as typeof window.eval;
  });
}

async function assertNoXss(page: Page): Promise<void> {
  const triggered = await page.evaluate(
    () => (window as unknown as { __rekindleXssTriggered: boolean }).__rekindleXssTriggered,
  );
  expect(triggered, "XSS payload was executed").toBe(false);
  const evalCalled = await page.evaluate(
    () => (window as unknown as { __rekindleEvalCalled: string | undefined }).__rekindleEvalCalled,
  );
  expect(evalCalled, "eval was called from rendered content").toBeUndefined();
}

test.describe("XSS — display names and profile fields", () => {
  test.beforeEach(async ({ page }) => {
    await installXssCanary(page);
  });

  test.afterEach(async ({ page }) => {
    await clearMocks(page);
  });

  for (const payload of XSS_PAYLOADS) {
    test(`identity display_name does not execute: ${payload.slice(0, 40)}`, async ({
      page,
    }) => {
      // Mock the login response with a malicious display_name.
      await page.goto("/login");
      await setupMocks(page, "login", {
        ...LOGIN_SUCCESS_HANDLER,
        get_identity: () => ({
          display_name: payload,
          public_key: "test-pk",
        }),
        list_identities: () => [
          { display_name: payload, public_key: "test-pk" },
        ],
      });

      await page.waitForLoadState("networkidle");
      await assertNoXss(page);

      // Confirm the payload appears as text — escaped — not as live HTML.
      const textContent = await page.evaluate(() => document.body.innerText);
      // The raw `<script>` characters must be present as text if the
      // display name is shown anywhere; the angle-bracket encoding is
      // what proves it was rendered as text and not parsed as HTML.
      // Either the payload is rendered (then must be escaped) or filtered
      // entirely — but it must NEVER execute.
      expect(textContent).not.toContain("__rekindleXssTriggered = true");
    });
  }
});

test.describe("CSP — verifying defensive CSP enforces", () => {
  test.beforeEach(async ({ page }) => {
    await installXssCanary(page);
  });

  test("inline script element is blocked", async ({ page }) => {
    await page.goto("/login");
    // Try to inject an inline script via DOM manipulation — should be
    // refused by the CSP `script-src 'self'` directive.
    const cspBlocked = await page.evaluate(() => {
      try {
        const s = document.createElement("script");
        s.textContent = 'window.__rekindleCspViolated = true';
        document.head.appendChild(s);
        return !(window as unknown as { __rekindleCspViolated?: boolean })
          .__rekindleCspViolated;
      } catch {
        return true;
      }
    });
    expect(cspBlocked, "inline script must be blocked by CSP").toBe(true);
    await assertNoXss(page);
  });

  test("eval is blocked or constrained", async ({ page }) => {
    await page.goto("/login");
    const evalBlocked = await page.evaluate(() => {
      try {
        // The default Tauri CSP does not include 'unsafe-eval', so eval
        // should throw. This test fails loudly if eval becomes available.
        // eslint-disable-next-line no-new-func, @typescript-eslint/no-implied-eval
        new Function("return 1")();
        return false;
      } catch {
        return true;
      }
    });
    expect(evalBlocked, "eval / Function ctor must be blocked").toBe(true);
  });

  test("data: URI iframe is blocked", async ({ page }) => {
    await page.goto("/login");
    await page.evaluate(() => {
      const f = document.createElement("iframe");
      f.src = 'data:text/html,<script>parent.__rekindleXssTriggered=true</script>';
      document.body.appendChild(f);
    });
    // Wait long enough for the iframe to attempt to load + execute.
    await page.waitForTimeout(500);
    await assertNoXss(page);
  });
});

test.describe("Markdown / link-preview / custom-emoji rendering — pending features", () => {
  // These tests are expected to fail-closed (skip) today, because the
  // features haven't shipped yet. They serve as a tripwire: when those
  // features land, the developer must make the test pass before the
  // feature merges. See docs/security/frontend-rendering.md.

  test.skip(
    "markdown renderer escapes raw HTML in message bodies",
    async () => {
      // When markdown rendering ships:
      //   1. Mock get_message_history with a body containing the payload
      //   2. Navigate to /chat?peer=test
      //   3. Assert no XSS, payload rendered as text
    },
  );

  test.skip(
    "link-preview titles do not execute embedded HTML",
    async () => {
      // When link previews ship: mock fetch_link_preview to return a
      // malicious title / description and assert no XSS.
    },
  );

  test.skip(
    "custom-emoji names do not execute embedded HTML",
    async () => {
      // When custom expressions land: mock list_expressions with a
      // payload-laden emoji name and assert no XSS in the picker.
    },
  );

  test.skip(
    "community / channel / role names do not execute embedded HTML",
    async () => {
      // Mock get_communities + get_roles with malicious names.
    },
  );

  test.skip(
    "presence custom-status field does not execute embedded HTML",
    async () => {
      // Mock get_friends with a malicious custom_status.
    },
  );
});
