import type { Browser } from "webdriverio";
import type { Peer } from "./fleet.ts";

/** Publish one normally signed post through this fleet peer's test daemon. */
export function postSkippingLocalPermission(peer: Peer, channel: string, body: string): Promise<unknown> {
  return peer.cli(["webdriver-post-skipping-local-permission", "--", channel, body]);
}

/** Hold incoming governance ops for this community on the app under test. */
export async function holdGovernanceOps(app: Browser, community: string): Promise<void> {
  await governanceRequest(app, "hold-governance-ops", community);
}

/** Store the queued ops and start the app's usual governance rebuild. */
export async function releaseGovernanceOps(app: Browser, community: string): Promise<number> {
  const result = await governanceRequest(app, "release-governance-ops", community) as { released: number };
  return result.released;
}

async function governanceRequest(
  app: Browser,
  operation: "hold-governance-ops" | "release-governance-ops",
  community: string,
): Promise<unknown> {
  const { port, headers: driverHeaders } = app.options;
  if (!port) throw new Error("the WebDriver session has no port");
  const headers = new Headers(driverHeaders);
  headers.set("Content-Type", "application/json");
  const response = await fetch(`http://127.0.0.1:${port}/wdio/${operation}`, {
    method: "POST",
    headers,
    body: JSON.stringify({ community }),
  });
  if (!response.ok) throw new Error(`${operation} failed with HTTP ${response.status}`);
  const answer = await response.json() as { Ok?: { data: unknown }; Error?: { message: string } };
  if (!answer.Ok) throw new Error(`${operation} failed: ${answer.Error?.message ?? "unexpected Host response"}`);
  return answer.Ok.data;
}
