// E7.14 harness: a script executed in a window the app has just destroyed
// and re-created under the same label answers or fails, never hangs. The
// menu's Profile re-creates the open Settings window under its label
// (`showSettings`); an `execute/sync` that landed before the new document's
// navigation committed used to write its result to a page global the
// navigation then wiped, and its poll waited out the 90 s script timeout
// (Linux, 2026-11-27). The result now comes back from the evaluation itself
// (`call_async_function`). Eight re-creates, each followed at once by an
// execute in the Settings window: every execute returns, a value or a
// JavaScript error, within 10 s.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, inBuddy, menu, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";

const b = browser;
const ROUNDS = 8;
const LIMIT_MS = 10_000;

describe("an execute in a window re-created under its label never hangs (E7.14)", function () {
  this.timeout(900_000);
  const ada: User = { ...newUser("recreate"), displayName: "Ada" };

  it("answers or fails within 10 s across eight Settings re-creates", async () => {
    await createIdentity(b, ada);
    await openSettings(b, "Appearance");
    const slow: string[] = [];
    for (let round = 1; round <= ROUNDS; round++) {
      await switchTo(b, BUDDY);
      await inBuddy(b);
      await menu(b, "Rekindle", "Profile");
      await switchTo(b, SETTINGS);
      const started = Date.now();
      let outcome: string;
      try {
        const title = (await b.execute(() => document.title)) as string;
        outcome = `value ${JSON.stringify(title)}`;
      } catch (e) {
        outcome = `error ${(e instanceof Error ? e.message : String(e)).split("\n")[0].slice(0, 160)}`;
      }
      const elapsed = Date.now() - started;
      console.log(`[recreate] round ${round}: ${elapsed} ms, ${outcome}`);
      if (elapsed > LIMIT_MS) slow.push(`round ${round}: ${elapsed} ms (${outcome})`);
    }
    expect(slow).toEqual([]);
  });
});
