// Plan E7.14 S1: create an identity in the real app. The Host makes the
// keys, the vault and the database, the Veilid node attaches (Create stays
// disabled until it can), and the buddy list opens with the new name.

import { browser } from "@wdio/globals";
import { createIdentity, newUser } from "../../journeys.ts";

describe("identity", () => {
  it("creates an identity and opens the buddy list under its name", async () => {
    await createIdentity(browser, newUser("wdio"));
  });
});
