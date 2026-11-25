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
import { chmodSync, chownSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
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
  /**
   * `args` run once without any confirmation: `ran` with its answer when it
   * needed none, else the refusal it printed (a reviewed command prints its
   * review, D3.4). Never runs a command twice.
   */
  review(args: string[]): Promise<Reviewed>;
  stop(): Promise<void>;
  /** For the process's `exit` hook, where only synchronous work runs. */
  stopSync(): void;
  /** Stop this peer's daemon and keep its HOME: the peer is offline, its identity and state kept. */
  pause(): Promise<void>;
  /** Start a paused peer's daemon again, unlock its identity, and wait until it is attached. */
  resume(): Promise<void>;
}

const started = new Set<Peer>();

/** What `Peer.review` saw: the command ran (its answer), or it was refused (what it printed). */
export type Reviewed = { ran: true; value: unknown } | { ran: false; refusal: { error?: { message?: string } } | null };

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

/**
 * The live rekindled this HOME's node lock names, else null. rekindle-db
 * lock.rs writes `std::process::id()` into `DataRoot.data/node.lock`
 * (`~/Library/Application Support/com.rekindle.app` on macOS,
 * `<XDG_DATA_HOME>/com.rekindle.app` elsewhere, rekindle-utils paths.rs).
 */
function lockHolder(home: string): number | null {
  const data = process.platform === "darwin"
    ? path.join(home, "Library", "Application Support", "com.rekindle.app")
    : path.join(home, ".local", "share", "com.rekindle.app");
  const lock = path.join(data, "node.lock");
  if (!existsSync(lock)) return null;
  const pid = Number.parseInt(readFileSync(lock, "utf8").trim(), 10);
  if (!Number.isInteger(pid) || pid <= 1) return null;
  const comm = spawnSync("ps", ["-p", String(pid), "-o", "comm="], { encoding: "utf8" }).stdout.trim();
  return path.basename(comm) === "rekindled" ? pid : null;
}

/**
 * A daemon that never opened its bus does not hear `node stop`, and its
 * HOME must not go while it still writes there. On macOS a 10 s stack
 * sample of it (`sample`, which only reads the process) goes beside its
 * kept logs, showing where its startup waits; then SIGTERM, and SIGKILL if
 * it has not exited 30 s later. Only the rekindled this HOME's own lock
 * names is signalled. True once no daemon holds the HOME.
 */
async function endStray(home: string, displayName: string): Promise<boolean> {
  const pid = lockHolder(home);
  if (pid === null) return true;
  const keep = path.join(KEEP_LOGS, displayName.slice(0, 24));
  mkdirSync(keep, { recursive: true });
  if (process.platform === "darwin") {
    await run("sample", [String(pid), "10", "-file", path.join(keep, "rekindled.sample.txt")]).catch(() => undefined);
  }
  const gone = async (): Promise<boolean> => lockHolder(home) !== pid;
  process.kill(pid, "SIGTERM");
  if (await waitUntil(gone, 30_000, "").then(() => true, () => false)) return true;
  process.kill(pid, "SIGKILL");
  return waitUntil(gone, 10_000, "").then(() => true, () => false);
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

  const review = async (args: string[]): Promise<Reviewed> => {
    try {
      const { stdout } = await run(exe, ["--format", "json", "--no-input", ...args], { env, maxBuffer: 16 * 1024 * 1024 });
      // It asked no review: it has run, and this is its answer.
      return { ran: true, value: lastJson(stdout) };
    } catch (err) {
      // A refused reviewed run prints its review on one line, then the
      // error (D3.4); a refusal before any review prints only the error.
      const out = ((err as { stdout?: string }).stdout ?? "").trim();
      const first = out.split("\n")[0] ?? "";
      if (first.length > 1 && first.startsWith("{")) return { ran: false, refusal: JSON.parse(first) };
      return { ran: false, refusal: out.startsWith("{") ? JSON.parse(out) : null };
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
  const attached = async (): Promise<void> => {
    await waitUntil(
      async () => {
        const status = (await cli(["status"]).catch(() => null)) as { attachment?: string; public_internet_ready?: boolean } | null;
        // Attached is not enough to write: the Host refuses a reviewed write
        // ("Not connected to the network yet") until the public internet is
        // ready (create_leave.rs NOT_CONNECTED, node.is_ready()), which the
        // daemon's status reports (StatusSnapshot.public_internet_ready).
        const attachedEnough = ["AttachedGood", "AttachedStrong", "FullyAttached", "OverAttached"].includes(status?.attachment ?? "");
        return attachedEnough && status?.public_internet_ready === true;
      },
      NET_WAIT_MS,
      `peer ${displayName} never attached with the public internet ready`,
    );
  };
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
      const ended = await endStray(home, displayName);
      keepLogs(home, displayName);
      if (!ended) throw new Error(`peer ${displayName}: rekindled ${lockHolder(home)} outlived SIGKILL; its HOME ${home} is kept`);
      rmSync(home, { recursive: true, force: true });
    },
    async pause() {
      await run(exe, ["--no-input", "node", "stop"], { env });
      await waitUntil(async () => !(await daemonRunning()), 30_000, `peer ${displayName} did not stop`);
    },
    async resume() {
      await run(exe, ["--no-input", "node", "start"], { env });
      await cli(["unlock", "--passphrase-file", passFile]);
      await attached();
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
      // Here only synchronous work runs: a daemon still holding the lock is
      // sent SIGTERM and its HOME left for the next run's sweep.
      const pid = lockHolder(home);
      if (pid !== null) {
        process.kill(pid, "SIGTERM");
        return;
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
    await attached();
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
  const reviewed = await peer.review(["community", "join", "--invite", invite]);
  if (reviewed.ran) throw new Error(`${peer.displayName}: the join ran without the fingerprint review it must ask`);
  const fingerprint = /--confirm=([0-9a-f ]{64,79})$/.exec(reviewed.refusal?.error?.message ?? "")?.[1];
  if (!fingerprint) throw new Error(`${peer.displayName}: the join asked for no fingerprint (${JSON.stringify(reviewed.refusal).slice(0, 200)})`);
  await peer.cli([`--confirm=${fingerprint}`, "community", "join", "--invite", invite]);
}

/** `peer` creates community `name` (reviewed: confirmed by its name, ADR 0019); its anchor key. */
export async function peerCreateCommunity(peer: Peer, name: string): Promise<string> {
  const made = (await peer.cli([`--confirm=${name}`, "community", "create", name])) as { anchor?: string } | null;
  if (!made?.anchor) throw new Error(`${peer.displayName}: community create answered no anchor (${JSON.stringify(made).slice(0, 200)})`);
  return made.anchor;
}

/** A `uses`-use invite link to community `anchor` from `peer`. */
export async function peerInvite(peer: Peer, anchor: string, uses = 1): Promise<string> {
  // Reviewed (ADR 0019): confirmed by the subject its review names.
  const made = (await peerReviewed(peer, ["community", "invite", "create", "--community", anchor, "--max-uses", String(uses)])) as { url?: string } | null;
  if (!made?.url) throw new Error(`${peer.displayName}: invite create answered no url (${JSON.stringify(made).slice(0, 200)})`);
  return made.url;
}

/** How many members `peer`'s view of community `anchor` counts. */
/** `peer`'s own pseudonym in community `anchor` (community info's `ourPseudonym`): what the CLI's member commands take. */
export async function peerOwnPseudonym(peer: Peer, anchor: string): Promise<string> {
  const info = (await peer.cli(["community", "info", "--community", anchor])) as { ourPseudonym?: string; our_pseudonym?: string } | null;
  const key = info?.ourPseudonym ?? info?.our_pseudonym ?? "";
  if (!/^[0-9a-f]{64}$/i.test(key)) throw new Error(`${peer.displayName}: community info served no pseudonym for ${anchor.slice(0, 16)}… (${JSON.stringify(info).slice(0, 200)})`);
  return key;
}

export async function peerMemberCount(peer: Peer, anchor: string): Promise<number> {
  const info = (await peer.cli(["community", "info", "--community", anchor])) as { member_count?: number; memberCount?: number } | null;
  return info?.member_count ?? info?.memberCount ?? 0;
}

/**
 * `args` as `peer`, answering its review when it asks one (D3.4): the CLI
 * names the subject to confirm (`--confirm=<subject>`), as a CLI user reads
 * it from their own review. A command that asks none runs as is.
 */
export async function peerReviewed(peer: Peer, args: string[]): Promise<unknown> {
  // A command that asks no review has run once already: its answer, never a
  // second run (that duplicated posts and invites). Otherwise the flag the
  // CLI itself names; a refusal naming none is that refusal.
  const reviewed = await peer.review(args);
  if (reviewed.ran) return reviewed.value;
  const message = reviewed.refusal?.error?.message ?? "";
  const subject = /--confirm=(.+)$/.exec(message)?.[1]?.trim();
  if (subject) return peer.cli([`--confirm=${subject}`, ...args]);
  if (/pass --yes/.test(message)) return peer.cli(["--yes", ...args]);
  throw new Error(`${peer.displayName}: ${args.join(" ")} was refused: ${message || JSON.stringify(reviewed.refusal).slice(0, 300)}`);
}

/** `peer` creates role `name` with permission bits `permissions` in community `anchor`; its role id. */
export async function peerCreateRole(peer: Peer, anchor: string, name: string, permissions: number): Promise<string> {
  const made = (await peerReviewed(peer, ["community", "role", "create", "--community", anchor, "--name", name, "--permissions", String(permissions)])) as Record<string, unknown> | null;
  const id = (made?.roleId ?? made?.role_id ?? made?.id) as string | undefined;
  if (!id) throw new Error(`${peer.displayName}: role create answered no id (${JSON.stringify(made).slice(0, 200)})`);
  return id;
}

/** `peer` gives member `member` (pseudonym or display name) role `roleId` in community `anchor`. */
export async function peerAssignRole(peer: Peer, anchor: string, member: string, roleId: string): Promise<void> {
  await peerReviewed(peer, ["community", "role", "assign", "--community", anchor, "--member", member, "--role-id", roleId]);
}

/** `peer` bans member `member` (pseudonym or display name) from community `anchor`. */
export async function peerBan(peer: Peer, anchor: string, member: string): Promise<void> {
  await peerReviewed(peer, ["community", "ban", "--community", anchor, "--member", member]);
}

/** `peer` takes role `roleId` back from member `member` in community `anchor`. */
export async function peerUnassignRole(peer: Peer, anchor: string, member: string, roleId: string): Promise<void> {
  await peerReviewed(peer, ["community", "role", "unassign", "--community", anchor, "--member", member, "--role-id", roleId]);
}

/** `peer` sends `text` to group `groupId` as a reply to the message whose client id is `replyTo` (CLI `--reply-to`, plan G-replies). */
export async function peerReplyGroup(peer: Peer, groupId: string, text: string, replyTo: string): Promise<void> {
  await peer.cli(["group", "send", groupId, text, "--reply-to", replyTo]);
}

/** `peer` kicks member `member` (pseudonym or display name) from community `anchor`. */
export async function peerKick(peer: Peer, anchor: string, member: string): Promise<void> {
  await peerReviewed(peer, ["community", "kick", "--community", anchor, "--member", member]);
}

/** `peer` posts `text` in channel `channel` of community `community` (names or keys). */
export async function peerPostChannel(peer: Peer, community: string, channel: string, text: string): Promise<void> {
  await peer.cli(["channel", "send", "-c", community, "-C", channel, "-m", text]);
}
