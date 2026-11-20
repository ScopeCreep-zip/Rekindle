// Plan E7.14 S4 (decided 2026-11-19, "S4/S5 data is real"): headless real
// peers for the lists whose rows are other people. Each peer is a
// `rekindled` with its own HOME, driven by the `rekindle` CLI (ADR 0010's
// thin frontend over the same Host), on the public Veilid network.
//
// Kept small and polite: peers start one at a time, each waiting until it
// is attached before the next, and every peer is stopped and its HOME
// removed at the end of a run (and on any exit), under the fixed prefix
// `/tmp/rkf.`.
//
// Each peer's HOME is short and owned by the user's primary group:
// macOS's `sun_path` holds 104 bytes, so the per-user $TMPDIR is too long
// for `<HOME>/Library/Application Support/rekindle/daemon.sock`, and
// Veilid's table store refuses a directory whose group is not the user's
// ("path has incorrect owner/group"), which /tmp's group (wheel) is not.
// On Linux the socket lives under XDG_RUNTIME_DIR (rekindle-ipc
// transport.rs), so every XDG directory is set inside the HOME too.

import { execFile, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { chmodSync, chownSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { promisify } from "node:util";
import { NET_WAIT_MS, waitUntil } from "../walk/helpers.ts";
import { cargoTargetDir } from "../walk/launch.ts";

const run = promisify(execFile);

/** The prefix every peer HOME is made under; a run removes what it made. */
const FLEET_PREFIX = "/tmp/rkf.";

export interface Peer {
  displayName: string;
  publicKey: string;
  /** `rekindle --format json --no-input <args>` as this peer; the parsed answer. */
  cli(args: string[]): Promise<unknown>;
  /** The review a reviewed command prints when run without its confirmation (D3.4), or null. */
  review(args: string[]): Promise<unknown>;
  stop(): Promise<void>;
  /** For the process's `exit` hook, where only synchronous work runs. */
  stopSync(): void;
}

const started = new Set<Peer>();

/**
 * The last JSON value a CLI run printed. A reviewed action prints its
 * review (one line) before its answer, so a run's output can hold two.
 */
function lastJson(stdout: string): unknown {
  const text = stdout.trim();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    const start = Math.max(text.lastIndexOf("\n{"), text.lastIndexOf("\n["));
    return JSON.parse(start >= 0 ? text.slice(start + 1) : text);
  }
}

/**
 * The fingerprint a trust-changing review names for a public key: its hex
 * in groups of four (the CLI's `--confirm` subject, e.g. "bc84 cec2 …").
 */
function fingerprint(publicKey: string): string {
  return (publicKey.match(/.{1,4}/g) ?? []).join(" ");
}
let exitHooked = false;

/** Where a run keeps each peer's logs (e2e/app/out/keep/fleet/<run start>/<peer>/, ignored by git). */
const KEEP_LOGS = path.join(import.meta.dirname, "out", "keep", "fleet", new Date().toISOString().replace(/[:.]/g, "-"));

/**
 * Copy a stopping peer's daemon and CLI logs out of its HOME before the
 * HOME goes, so a failure can be traced from the peer's side too. The log
 * directory is `DataRoot`'s (rekindle-utils paths.rs): `~/Library/Logs/
 * com.rekindle.app` on macOS, `<XDG_DATA_HOME>/com.rekindle.app/logs`
 * elsewhere.
 */
function keepLogs(home: string, displayName: string): void {
  const logs = process.platform === "darwin"
    ? path.join(home, "Library", "Logs", "com.rekindle.app")
    : path.join(home, ".local", "share", "com.rekindle.app", "logs");
  if (!existsSync(logs)) return;
  cpSync(logs, path.join(KEEP_LOGS, displayName.slice(0, 24)), { recursive: true });
}

/** The CLI beside the app the suite launches: the same cargo target directory (launch.ts). */
function cliPath(): string {
  return path.join(cargoTargetDir(), "debug", "rekindle");
}

function peerEnv(home: string): NodeJS.ProcessEnv {
  const run = path.join(home, "run");
  mkdirSync(run, { recursive: true, mode: 0o700 });
  return {
    ...process.env,
    HOME: home,
    XDG_RUNTIME_DIR: run,
    XDG_DATA_HOME: path.join(home, ".local", "share"),
    XDG_CONFIG_HOME: path.join(home, ".config"),
    XDG_STATE_HOME: path.join(home, ".local", "state"),
    XDG_CACHE_HOME: path.join(home, ".cache"),
  };
}

/**
 * Start one peer: its own HOME, `rekindle node start`, a new identity named
 * `displayName`, then wait until the CLI reports it attached.
 */
async function startPeer(displayName: string): Promise<Peer> {
  const home = mkdtempSync(FLEET_PREFIX);
  chownSync(home, process.getuid?.() ?? 0, process.getgid?.() ?? 0);
  writeFileSync(path.join(home, MARKER), "");
  const env = peerEnv(home);
  const passFile = path.join(home, "pass");
  writeFileSync(passFile, randomBytes(18).toString("base64url"), { mode: 0o600 });
  chmodSync(passFile, 0o600);
  const exe = cliPath();

  const review = async (args: string[]): Promise<unknown> => {
    try {
      await run(exe, ["--format", "json", "--no-input", ...args], { env, maxBuffer: 16 * 1024 * 1024 });
      return null;
    } catch (err) {
      // A refused reviewed run prints its review on one line, then the
      // error (D3.4); a refusal before any review prints only the error.
      const out = ((err as { stdout?: string }).stdout ?? "").trim();
      const first = out.split("\n")[0] ?? "";
      if (first.length > 1 && first.startsWith("{")) return JSON.parse(first);
      return out.startsWith("{") ? JSON.parse(out) : null;
    }
  };

  const cli = async (args: string[]): Promise<unknown> => {
    try {
      const { stdout } = await run(exe, ["--format", "json", "--no-input", ...args], { env, maxBuffer: 16 * 1024 * 1024 });
      return lastJson(stdout);
    } catch (err) {
      // The CLI prints a review line and then `{"error": {...}}` (D3.4); name its message.
      const out = (err as { stdout?: string }).stdout ?? "";
      const said = (lastJson(out) as { error?: { message?: string } } | null)?.error?.message;
      throw new Error(`${displayName}: rekindle ${args.join(" ").slice(0, 80)} failed: ${said ?? (err instanceof Error ? err.message : String(err))}`);
    }
  };

  const daemonRunning = async (): Promise<boolean> => {
    const status = (await cli(["status"]).catch(() => null)) as { daemon?: string } | null;
    return status?.daemon !== "not running";
  };

  let stopped = false;
  const peer: Peer = {
    displayName,
    publicKey: "",
    cli,
    review,
    async stop() {
      if (stopped) return;
      stopped = true;
      started.delete(peer);
      await run(exe, ["--no-input", "node", "stop"], { env }).catch(() => undefined);
      // `node stop` only starts the shutdown; the daemon writes into its HOME
      // until it has exited, so the HOME goes once status says so.
      await waitUntil(async () => !(await daemonRunning()), 30_000, `peer ${displayName} did not stop`).catch(() => undefined);
      keepLogs(home, displayName);
      rmSync(home, { recursive: true, force: true });
    },
    stopSync() {
      if (stopped) return;
      stopped = true;
      started.delete(peer);
      spawnSync(exe, ["--no-input", "node", "stop"], { env, timeout: 10_000 });
      const deadline = Date.now() + 10_000;
      while (Date.now() < deadline) {
        const status = spawnSync(exe, ["--format", "json", "--no-input", "status"], { env, encoding: "utf8", timeout: 5_000 });
        if (/"daemon":\s*"not running"/.test(status.stdout ?? "")) break;
      }
      rmSync(home, { recursive: true, force: true });
    },
  };
  started.add(peer);
  if (!exitHooked) {
    exitHooked = true;
    // Synchronous last resort (an abort, an uncaught error): stop each daemon, drop its HOME.
    process.once("exit", () => {
      for (const p of [...started]) p.stopSync();
    });
  }

  try {
    await run(exe, ["--no-input", "node", "start"], { env });
    const made = (await cli(["init", "--display-name", displayName, "--passphrase-file", passFile])) as { publicKey?: string } | null;
    const list = (await cli(["identity", "list"])) as { publicKey: string }[];
    peer.publicKey = made?.publicKey ?? list[0]?.publicKey ?? "";
    if (!peer.publicKey) throw new Error(`peer ${displayName}: no identity after init`);
    await waitUntil(
      async () => {
        const status = (await cli(["status"]).catch(() => null)) as { attachment?: string } | null;
        // veilid-core AttachmentState (veilid_state.rs): good enough to use the network.
        return ["AttachedGood", "AttachedStrong", "FullyAttached", "OverAttached"].includes(status?.attachment ?? "");
      },
      NET_WAIT_MS,
      `peer ${displayName} never attached`,
    );
  } catch (err) {
    await peer.stop();
    throw err;
  }
  return peer;
}

/** Start `names.length` peers one after another (each attached before the next). */
/** The file a fleet HOME holds from its first moment: only such a HOME is ever swept. */
const MARKER = ".rekindle-fleet-home";

/**
 * A run that was killed (SIGKILL, a crash) never reached its peers'
 * stop(); their HOMEs stay under the prefix. Before starting, remove each
 * HOME this suite made (it holds `MARKER`) whose daemon is not running,
 * keeping its logs first. Anything else under the prefix is left alone.
 */
function sweepStaleHomes(): void {
  const parent = path.dirname(FLEET_PREFIX);
  const stem = path.basename(FLEET_PREFIX);
  for (const name of readdirSync(parent)) {
    if (!name.startsWith(stem)) continue;
    const home = path.join(parent, name);
    if (!existsSync(path.join(home, MARKER))) continue;
    const status = spawnSync(cliPath(), ["--format", "json", "--no-input", "status"], { env: peerEnv(home), encoding: "utf8", timeout: 10_000 });
    if (!/"daemon":\s*"not running"/.test(status.stdout ?? "")) continue;
    keepLogs(home, `stale-${name}`);
    rmSync(home, { recursive: true, force: true });
  }
}

let swept = false;

export async function startFleet(names: string[]): Promise<Peer[]> {
  if (!swept) {
    swept = true;
    sweepStaleHomes();
  }
  const peers: Peer[] = [];
  try {
    for (const name of names) peers.push(await startPeer(name));
  } catch (err) {
    await stopFleet(peers);
    throw err;
  }
  return peers;
}

export async function stopFleet(peers: Peer[]): Promise<void> {
  await Promise.all(peers.map((p) => p.stop()));
}

/** This peer's friend code URL (`rekindle friend code`). */
export async function peerFriendCode(peer: Peer): Promise<string> {
  const code = (await peer.cli(["friend", "code"])) as { url: string };
  return code.url;
}

/** `peer` sends a friend request to `url` with `message`. */
export async function peerRequestFriend(peer: Peer, url: string, message: string): Promise<void> {
  // Trust-changing (ADR 0019): confirmed by naming the fingerprint of the
  // key the code holds, as a user checks the review against whom they add.
  const key = /^rekindle:\/\/add\/([0-9a-f]{64})\//.exec(url)?.[1];
  if (!key) throw new Error(`not a friend code: ${url.slice(0, 40)}`);
  await peer.cli([`--confirm=${fingerprint(key)}`, "friend", "add", url, "--message", message]);
}

/** `peer` accepts the pending request from `publicKey`, once it has arrived. */
export async function peerAcceptFrom(peer: Peer, publicKey: string): Promise<void> {
  await waitUntil(
    async () => {
      const requests = (await peer.cli(["friend", "requests"])) as { publicKey?: string }[];
      return requests.some((r) => r.publicKey === publicKey);
    },
    NET_WAIT_MS,
    `${peer.displayName} never saw the request`,
  );
  await peer.cli([`--confirm=${fingerprint(publicKey)}`, "friend", "accept", "--request-id", publicKey]);
}

/** `peer` sends `text` to friend `to` (a public key). */
export async function peerSendDm(peer: Peer, to: string, text: string): Promise<void> {
  await peer.cli(["dm", "send", "--friend", to, "--message", text]);
}

/** One of `rekindle group list`'s rows (rekindle-types dto/group_dm.rs `GroupDmView`). */
interface GroupRow {
  groupId: string;
  name: string;
  pending: boolean;
}

/** `peer` accepts the pending invite to group `name`, once it has arrived; the group's id. */
export async function peerAcceptGroup(peer: Peer, name: string): Promise<string> {
  let id = "";
  await waitUntil(
    async () => {
      const rows = (await peer.cli(["group", "list"])) as GroupRow[];
      id = rows.find((g) => g.pending && g.name === name)?.groupId ?? "";
      return id !== "";
    },
    NET_WAIT_MS,
    `${peer.displayName} never saw the invite to ${name.slice(0, 24)}`,
  );
  // Trust-changing (ADR 0019): confirmed by naming the group, as its review asks.
  await peer.cli([`--confirm=${name}`, "group", "accept", id]);
  return id;
}

/** `peer` posts `text` to group `groupId`. */
export async function peerSendGroup(peer: Peer, groupId: string, text: string): Promise<void> {
  await peer.cli(["group", "send", groupId, text]);
}

/**
 * `peer` joins by `invite`. The join is reviewed (ADR 0019) and confirmed by
 * the invite's 64-hex fingerprint its review prints (cli/community.rs
 * `Join`: "the 64 hex digits the review prints"); no inviter screen shows
 * that fingerprint to compare against, so, as a CLI user does, the peer
 * confirms the fingerprint its own review names.
 */
export async function peerJoinCommunity(peer: Peer, invite: string): Promise<void> {
  const refused = (await peer.review(["community", "join", "--invite", invite])) as { error?: { message?: string } } | null;
  const fingerprint = /--confirm=([0-9a-f ]{64,79})$/.exec(refused?.error?.message ?? "")?.[1];
  if (!fingerprint) throw new Error(`${peer.displayName}: the join asked for no fingerprint (${JSON.stringify(refused).slice(0, 200)})`);
  await peer.cli([`--confirm=${fingerprint}`, "community", "join", "--invite", invite]);
}
