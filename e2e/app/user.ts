// Plan E7.14: the throwaway identity every real-app journey drives (a leaf,
// so the journey modules share it without importing each other).

import { randomBytes } from "node:crypto";

/** A throwaway identity: its name and passphrase live only in this run. */
export interface User {
  displayName: string;
  passphrase: string;
}

export function newUser(prefix: string): User {
  return { displayName: `${prefix}-${randomBytes(3).toString("hex")}`, passphrase: randomBytes(18).toString("base64url") };
}
