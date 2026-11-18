// Plan E7.13: the community voice-channel rows (M104, M12, M106, M107,
// M84, M85, M114, M25 of
// .claude/plans/standards-remediation/evidence/app-journeys-2026-11-17.md),
// walked with the peer (rows-peer.ts PEER0) after it joined the walk's
// community (rows-peer-community.ts B61) and before it is kicked (M86).
//
// Joining opens the microphone at once: join_voice_channel → start_session
// (rekindle-voice session/start.rs:51,84) → init_voice_session_impl
// (src-tauri voice_adapter/session_setup.rs:96 `start_capture`) →
// AudioCapture::start (rekindle-voice capture.rs:147 `build_input_stream`).
// A failed open is non-fatal (listen-only, session_setup.rs:85-103), but on
// macOS the open itself is a TCC microphone check the walk must never
// trigger, so every row that joins skips there. Elsewhere it opens the
// default input device, or joins listen-only when the open fails.

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, Skip, WAIT_MS, byText, gone, present, press, resolveIntent, shown, typeInto, waitUntil } from "./helpers.ts";
import { setValue } from "./nav.ts";
import { contextMenu, inCommunity } from "./rows-community.ts";
import { peerMember } from "./rows-peer-community.ts";
import { needPeer } from "./rows-peer.ts";
import type { Row } from "./rows.ts";

const VOICE_CH = "walk-call";

function needCapture(): void {
  if (process.platform === "darwin") {
    throw new Skip(
      "[MIC] macOS: joining opens the microphone (session_setup.rs:96 start_capture → capture.rs:147 build_input_stream), a TCC check the walk must not trigger",
    );
  }
}

/** Select the walk's voice channel in the open community window. */
async function selectVoice(b: Browser, community: string): Promise<void> {
  await inCommunity(b, community);
  await (await byText(b, ".channel-item", VOICE_CH, NET_WAIT_MS)).click();
}

/**
 * The channel row's phone button. It is opacity 0 until the row is hovered
 * (theme/08-community-events.css:2-4), and a page-dispatched event sets no
 * :hover, so it is found in the row and pressed in the page (helpers.ts press).
 */
async function rowJoinButton(b: Browser): Promise<Awaited<ReturnType<Browser["$"]>>> {
  const row = await byText(b, ".channel-item", VOICE_CH, NET_WAIT_MS);
  const btn = await row.$(`[aria-label="Join voice channel ${VOICE_CH}"]`);
  if (!(await btn.isExisting())) throw new Error(`the ${VOICE_CH} row has no Join Voice button`);
  return btn;
}

/** Fail with the toast handleJoinVoice shows when the Host refuses (voice.actions.ts:140-144). */
async function joinRefusal(b: Browser): Promise<string> {
  for (const t of await b.$$(".toast-item.toast-error")) {
    const text = await t.getText();
    if (text.includes("Couldn't join voice")) return text;
  }
  return "";
}

/** The names in the sidebar voice panel (VoicePanel.tsx:75-80, VoiceParticipant.tsx:14-16). */
async function voiceNames(b: Browser): Promise<string> {
  const names = await b.$$(".voice-panel .voice-participant-name");
  const out: string[] = [];
  for (const n of names) out.push(await n.getText());
  return out.join(" | ");
}

async function waitListed(b: Browser, name: string, listed: boolean, who: string): Promise<void> {
  await waitUntil(
    async () => (await voiceNames(b)).includes(name) === listed,
    NET_WAIT_MS,
    `${who}'s voice panel ${listed ? "never listed" : "still lists"} ${name} (lists: ${await voiceNames(b)})`,
  );
}

/** Click CallStage's Join hero button and wait for the connected stage (CallStage.tsx:22-40). */
async function joinFromStage(b: Browser): Promise<void> {
  await (await shown(b, ".call-stage-join-btn", WAIT_MS)).click();
  let refused = "";
  await waitUntil(
    async () => {
      refused = await joinRefusal(b);
      return refused !== "" || (await present(b, ".call-stage-body"));
    },
    NET_WAIT_MS,
    "the call stage never connected and no refusal toast showed",
  );
  if (refused) throw new Error(refused);
  await shown(b, ".voice-panel", WAIT_MS);
}

async function pressedState(b: Browser, selector: string): Promise<string> {
  return (await (await shown(b, selector)).getAttribute("aria-pressed")) ?? "none";
}

/** Server Mute / Server Deafen from the peer's member menu (MemberList.tsx:238-270). */
async function serverAction(b: Browser, community: string, item: string, passphrase: string): Promise<string> {
  await inCommunity(b, community);
  await contextMenu(b, await peerMember(b), item);
  await resolveIntent(b, { passphrase, appearMs: 1500 });
  // handleServerMuteMember / handleServerDeafenMember toast their outcome
  // (actions/community/moderation.ts:79-85).
  let toast = "";
  await waitUntil(
    async () => {
      for (const t of await b.$$(".toast-item")) {
        const text = await t.getText();
        if (/server-(muted|deafened)|Failed to update server-/i.test(text)) toast = text;
      }
      return toast !== "";
    },
    WAIT_MS,
    `no outcome toast after ${item}`,
  );
  if (toast.startsWith("Failed")) throw new Error(toast);
  return toast;
}

export const VOICE_ROWS: Row[] = [
  {
    id: "M10v",
    title: "Create voice channel walk-call; selecting it shows the Join hero (no join)",
    async run({ b, s, shot }) {
      await inCommunity(b, s.communityName);
      await (await shown(b, '[title="Create Channel"]')).click();
      await typeInto(await shown(b, 'input[placeholder="Channel name..."]'), VOICE_CH);
      await setValue(b, ".modal-container select.form-select", "voice");
      await (await b.$('.modal-container button[type="submit"]')).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await gone(b, 'input[placeholder="Channel name..."]', NET_WAIT_MS);
      await selectVoice(b, s.communityName);
      const title = await (await shown(b, ".call-stage-hero .call-stage-hero-title", WAIT_MS)).getText();
      if (title !== VOICE_CH) throw new Error(`the Join hero names "${title}", not ${VOICE_CH}`);
      await shown(b, ".call-stage-join-btn");
      await rowJoinButton(b);
      if (await present(b, ".voice-panel")) throw new Error("the voice panel shows before any join");
      await shot();
      return "Join hero shown, the row holds its hover-revealed Join Voice button, no voice panel";
    },
  },
  {
    id: "M104",
    title: "Voice channel: Join (CallStage) — the walk joins walk-call",
    async run({ b, s, shot }) {
      needCapture();
      await selectVoice(b, s.communityName);
      await joinFromStage(b);
      await waitListed(b, s.displayName, true, "the walk");
      // No camera or screen share yet: the walk's own tile is the avatar
      // placeholder (ParticipantTile.tsx:47), both capture buttons unpressed.
      await shown(b, ".call-stage-gallery .call-stage-tile .call-stage-tile-avatar");
      const cam = await pressedState(b, '.call-stage [aria-label="Start camera"]');
      const screen = await pressedState(b, '.call-stage [aria-label="Share screen"]');
      if (cam !== "false" || screen !== "false") throw new Error(`capture buttons pressed: camera=${cam} screen=${screen}`);
      await shot();
      return "connected stage, own avatar tile, the walk listed in the voice panel";
    },
  },
  {
    id: "M12",
    title: "Peer joins walk-call from the channel row's phone button; both list each other",
    async run({ b, s, shot }) {
      needCapture();
      const p = needPeer();
      await inCommunity(p.b, s.communityName);
      await press(p.b, await rowJoinButton(p.b));
      let refused = "";
      await waitUntil(
        async () => {
          refused = await joinRefusal(p.b);
          return refused !== "" || (await present(p.b, ".voice-panel"));
        },
        NET_WAIT_MS,
        "the peer's voice panel never showed and no refusal toast",
      );
      if (refused) throw new Error(`peer: ${refused}`);
      await waitListed(p.b, s.displayName, true, "the peer");
      await waitListed(b, p.s.displayName, true, "the walk");
      await waitUntil(
        async () => (await (await b.$(".call-stage-gallery")).getText()).includes(p.s.displayName),
        NET_WAIT_MS,
        "the walk's call stage shows no tile for the peer",
      );
      await shot();
      await p.b.saveScreenshot("e2e/walk/out/M12-peer.png");
      return "each voice panel lists the other; the walk's stage has the peer's tile";
    },
  },
  {
    id: "M106",
    title: "Control bar: Mute, then unmute",
    async run({ b, s, shot }) {
      needCapture();
      await selectVoice(b, s.communityName);
      await (await shown(b, '.call-stage [aria-label="Mute microphone"]')).click();
      await waitUntil(async () => (await pressedState(b, '.call-stage [aria-label="Unmute microphone"]')) === "true", WAIT_MS, "the mute button never showed pressed");
      await shown(b, '.voice-panel [aria-label="Unmute microphone"]');
      await shot();
      await (await shown(b, '.call-stage [aria-label="Unmute microphone"]')).click();
      await waitUntil(async () => (await pressedState(b, '.call-stage [aria-label="Mute microphone"]')) === "false", WAIT_MS, "the mute button never released");
      return "control bar and voice panel agree: muted, then unmuted";
    },
  },
  {
    id: "M107",
    title: "Control bar: Deafen, then undeafen",
    async run({ b, s, shot }) {
      needCapture();
      await selectVoice(b, s.communityName);
      await (await shown(b, '.call-stage [aria-label="Deafen output"]')).click();
      await waitUntil(async () => (await pressedState(b, '.call-stage [aria-label="Undeafen output"]')) === "true", WAIT_MS, "the deafen button never showed pressed");
      await shown(b, '.voice-panel [aria-label="Undeafen output"]');
      await shot();
      await (await shown(b, '.call-stage [aria-label="Undeafen output"]')).click();
      await waitUntil(async () => (await pressedState(b, '.call-stage [aria-label="Deafen output"]')) === "false", WAIT_MS, "the deafen button never released");
      return "control bar and voice panel agree: deafened, then undeafened";
    },
  },
  {
    id: "M84",
    title: "Member menu > Server mute (the peer is in walk-call)",
    async run({ b, s, shot }) {
      needCapture();
      const toast = await serverAction(b, s.communityName, "Server Mute", s.passphrase);
      await shot();
      return toast;
    },
  },
  {
    id: "M85",
    title: "Member menu > Server deafen (the peer is in walk-call)",
    async run({ b, s, shot }) {
      needCapture();
      const toast = await serverAction(b, s.communityName, "Server Deafen", s.passphrase);
      await shot();
      return toast;
    },
  },
  {
    id: "M114",
    title: "Control bar: Disconnect; the peer stops listing the walk; the walk rejoins",
    async run({ b, s, shot }) {
      needCapture();
      const p = needPeer();
      await selectVoice(b, s.communityName);
      await (await shown(b, '.call-stage [aria-label="Leave voice channel"]')).click();
      await shown(b, ".call-stage-hero", NET_WAIT_MS);
      await gone(b, ".voice-panel", WAIT_MS);
      await waitListed(p.b, s.displayName, false, "the peer");
      await joinFromStage(b);
      await waitListed(p.b, s.displayName, true, "the peer");
      await waitListed(b, p.s.displayName, true, "the walk");
      await shot();
      return "left (hero back, peer dropped the walk), rejoined (peer lists the walk again)";
    },
  },
  {
    id: "M25",
    title: "Voice panel: Disconnect (walk), then the peer leaves from its control bar",
    async run({ b, s, shot }) {
      needCapture();
      const p = needPeer();
      await inCommunity(b, s.communityName);
      await (await shown(b, '.voice-panel [aria-label="Leave voice channel"]')).click();
      await gone(b, ".voice-panel", NET_WAIT_MS);
      await waitListed(p.b, s.displayName, false, "the peer");
      await inCommunity(p.b, s.communityName);
      await (await byText(p.b, ".channel-item", VOICE_CH, NET_WAIT_MS)).click();
      await (await shown(p.b, '.call-stage [aria-label="Leave voice channel"]')).click();
      await gone(p.b, ".voice-panel", NET_WAIT_MS);
      await shot();
      return "both left walk-call";
    },
  },
];
