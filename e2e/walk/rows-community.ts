// Plan E7.13: the single-identity Community rows (M*, CS*) of
// .claude/plans/standards-remediation/evidence/app-journeys-2026-11-17.md.
// One community window stays open across these rows; the community is the
// one rows.ts B60 created. Selectors come from windows/community_window/*
// and components/community/*; message actions show on hover only
// (theme/10-thread-list-panel.css:120-132), so they are pressed in the page.

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, Skip, WAIT_MS, byText, doubleClick, gone, present, press, resolveIntent, shown, sleep, typeAndSend, typeInto, waitForHandle, waitUntil } from "./helpers.ts";
import { inBuddy, setValue } from "./nav.ts";
import type { Row } from "./rows.ts";

const TEXT_CH = "walk-text";
const RENAMED_CH = "walk-renamed";
const CATEGORY = "walk-cat";
const MSG = "hello from the walk";
const EDITED = "edited by the walk";
const THREAD = "walk-thread";

/** Switch to the open community window, or open it from the buddy list. */
export async function inCommunity(b: Browser, name: string): Promise<void> {
  const open = (await b.getWindowHandles()).find((h) => h.startsWith("community-"));
  if (open) {
    await b.switchToWindow(open);
  } else {
    await inBuddy(b);
    await (await b.$('.buddy-tab[title^="Communities"]')).click();
    const before = await b.getWindowHandles();
    await doubleClick(b, await byText(b, ".community-item", name, NET_WAIT_MS));
    await b.switchToWindow(await waitForHandle(b, (h) => h.startsWith("community-") && !before.includes(h)));
  }
  await shown(b, ".community-sidebar", NET_WAIT_MS);
}

export async function contextMenu(b: Browser, el: Awaited<ReturnType<Browser["$"]>>, item: string): Promise<void> {
  await b.execute((node: HTMLElement) => {
    const r = node.getBoundingClientRect();
    node.dispatchEvent(
      new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: r.x + 5, clientY: r.y + 5 }),
    );
  }, el as unknown as HTMLElement);
  await shown(b, ".context-menu");
  await press(b, await byText(b, ".context-menu .context-menu-item", item));
}

async function modalSubmit(b: Browser, input: string, value: string, passphrase: string): Promise<void> {
  await typeInto(await shown(b, input), value);
  await (await b.$('.modal-container button[type="submit"]')).click();
  await resolveIntent(b, { passphrase, appearMs: 3000 });
  await gone(b, input, NET_WAIT_MS);
}

async function channel(b: Browser, name: string): Promise<Awaited<ReturnType<Browser["$"]>>> {
  return byText(b, ".channel-item", name, NET_WAIT_MS);
}

async function lastMessage(b: Browser, text: string): Promise<Awaited<ReturnType<Browser["$"]>>> {
  return byText(b, ".community-main .chat-message", text, NET_WAIT_MS);
}

async function msgAction(b: Browser, text: string, label: string): Promise<void> {
  const msg = await lastMessage(b, text);
  const btn = await msg.$(`[aria-label="${label}"]`);
  await btn.waitForExist({ timeout: WAIT_MS, timeoutMsg: `no ${label} on the message` });
  await press(b, btn);
}

/**
 * Records `edit_channel_message`'s IPC requests and answers in the page: M45
 * names them when the edit never shows, to tell a save that was never sent
 * or was refused from an event never applied. Tauri defines
 * `__TAURI_INTERNALS__.invoke` and `.ipc` read-only (tauri 2.12.1
 * scripts/core.js:81, ipc.js:142), so the hook is the `fetch` its IPC
 * protocol posts each call with (scripts/ipc-protocol.js:37).
 */
async function recordEditCalls(b: Browser): Promise<void> {
  await b.execute(() => {
    type W = { __walkEdits?: string[]; __walkFetch?: typeof fetch };
    const w = window as unknown as W;
    w.__walkEdits = [];
    if (w.__walkFetch) return;
    // Each Enter: where it landed and with what value, and whether a handler
    // took it (MessageInput's onKeyDown prevents both a save and a cancel).
    document.addEventListener("keydown", (e) => {
      if (e.key !== "Enter") return;
      const t = e.target as HTMLTextAreaElement;
      w.__walkEdits!.push(`Enter on ${t.className || t.tagName} value=${JSON.stringify(t.value ?? null)}`);
    }, true);
    window.addEventListener("keydown", (e) => {
      if (e.key === "Enter") w.__walkEdits!.push(`Enter prevented=${e.defaultPrevented}`);
    });
    const real = window.fetch.bind(window);
    w.__walkFetch = real;
    window.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const p = real(input, init);
      if (url.includes("edit_channel_message")) {
        const log = w.__walkEdits!;
        const at = log.push(`sent ${String(init?.body).slice(0, 300)}`) - 1;
        p.then(
          (r) => r.clone().text().then((t) => { log[at] += ` -> ${r.status} ${r.headers.get("Tauri-Response") ?? "?"} ${t.slice(0, 300)}`; }),
          (e: unknown) => { log[at] += ` -> fetch error ${String(e)}`; },
        );
      }
      return p;
    };
  });
}

async function editCalls(b: Browser): Promise<string> {
  return b.execute(() => {
    const w = window as unknown as { __walkEdits?: string[] };
    const ids = Array.from(document.querySelectorAll(".community-main .chat-message")).map((m) => m.textContent?.trim().slice(0, 40));
    const dialog = !!document.querySelector(".intent-dialog");
    const editing = !!document.querySelector(".community-main .message-input-editing");
    return `edit calls: ${(w.__walkEdits ?? []).join(" ; ") || "none"}; dialog open=${dialog}; editing=${editing}; messages: ${ids.join(" | ")}`;
  });
}

export async function openCommunitySettings(b: Browser, tab: string): Promise<void> {
  if (!(await present(b, ".modal-container .form-tabs"))) {
    await (await shown(b, '[title="Community Settings"]')).click();
    await shown(b, ".modal-container .form-tabs");
  }
  await (await byText(b, ".modal-container .form-tab", tab)).click();
  await waitUntil(async () => (await (await b.$(".modal-container .form-tab-active")).getText()).trim() === tab, WAIT_MS, `${tab} tab not active`);
}

export async function closeCommunityModal(b: Browser): Promise<void> {
  if (await present(b, ".modal-container .modal-close-btn")) {
    await (await b.$(".modal-container .modal-close-btn")).click();
    await gone(b, ".modal-container");
  }
}

export const COMMUNITY_ROWS: Row[] = [
  {
    id: "M4",
    title: "Select a community",
    async run({ b, s, shot }) {
      await inCommunity(b, s.communityName);
      await press(b, await byText(b, ".community-list .community-name", s.communityName, NET_WAIT_MS));
      await shown(b, ".channel-list", NET_WAIT_MS);
      await shot();
      return `channel list: ${(await (await b.$(".channel-list")).getText()).replace(/\s+/g, " ").slice(0, 80)}`;
    },
  },
  {
    id: "M3",
    title: "Sidebar: Create community",
    async run({ b, s, shot }) {
      await inCommunity(b, s.communityName);
      const second = `${s.communityName}-2`;
      await (await shown(b, '.community-sidebar [title="Create Community"]')).click();
      await modalSubmit(b, 'input[placeholder="Community name..."]', second, s.passphrase);
      await byText(b, ".community-list .community-name", second, NET_WAIT_MS);
      await shot();
      await press(b, await byText(b, ".community-list .community-name", s.communityName));
      return `${second} listed`;
    },
  },
  {
    id: "M10",
    title: "Create channel: text and voice",
    async run({ b, s, shot }) {
      await inCommunity(b, s.communityName);
      for (const [name, kind] of [[TEXT_CH, "text"], ["walk-voice", "voice"]] as const) {
        await (await shown(b, '[title="Create Channel"]')).click();
        await typeInto(await shown(b, 'input[placeholder="Channel name..."]'), name);
        await setValue(b, ".modal-container select.form-select", kind);
        await (await b.$('.modal-container button[type="submit"]')).click();
        await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
        await gone(b, 'input[placeholder="Channel name..."]', NET_WAIT_MS);
        await channel(b, name);
      }
      await shot();
      return "walk-text and walk-voice listed";
    },
  },
  {
    id: "M11",
    title: "Select a channel",
    async run({ b, s, shot }) {
      await inCommunity(b, s.communityName);
      await (await channel(b, TEXT_CH)).click();
      await waitUntil(async () => (await (await b.$(".community-channel-header-name")).getText()) === TEXT_CH, WAIT_MS, "header did not name the channel");
      await shot();
    },
  },
  {
    id: "M26",
    title: "Edit channel topic (Enter)",
    async run({ b, s, shot }) {
      const topic = "walk topic";
      await (await shown(b, ".channel-topic-editable")).click();
      await typeInto(await shown(b, ".channel-topic-header-input"), topic);
      await b.keys("Enter");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await waitUntil(async () => (await (await b.$(".channel-topic")).getText()).includes(topic), NET_WAIT_MS, "topic not shown");
      await shot();
    },
  },
  {
    id: "M42",
    title: "Send message",
    async run({ b, s, shot }) {
      await typeAndSend(b, ".community-main .message-input-field", MSG);
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await lastMessage(b, MSG);
      await shot();
    },
  },
  {
    id: "M45",
    title: "Edit own message > save",
    async run({ b, s, shot }) {
      await msgAction(b, MSG, "Edit message");
      const input = await shown(b, ".community-main .message-input-editing");
      await input.clearValue();
      await input.addValue(EDITED);
      await recordEditCalls(b);
      const typed = await input.getValue();
      await b.keys("Enter");
      const outcome = await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await lastMessage(b, EDITED).catch(async (e: Error) => {
        throw new Error(`${e.message} (field before Enter: ${JSON.stringify(typed)}; intent ${outcome}; ${await editCalls(b)})`);
      });
      await shot();
    },
  },
  {
    id: "M50",
    title: "Pin message, then M27 pins panel",
    async run({ b, s, shot }) {
      await msgAction(b, EDITED, "Pin message");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await (await shown(b, '[title="Pinned Messages"]')).click();
      await byText(b, ".pin-panel-item", EDITED, NET_WAIT_MS);
      await shot();
      return "pinned message listed in the pins panel";
    },
  },
  {
    id: "M74",
    title: "Pins: Unpin",
    async run({ b, s, shot }) {
      await (await shown(b, ".pin-panel-unpin-btn")).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await shown(b, ".pin-panel .pin-panel-empty", NET_WAIT_MS);
      await shot();
      await (await b.$('[aria-label="Close pinned messages panel"]')).click();
    },
  },
  {
    id: "M51",
    title: "Create thread, then M28 threads panel",
    async run({ b, s, shot }) {
      await msgAction(b, EDITED, "Create thread");
      await modalSubmit(b, 'input[placeholder="Thread name..."]', THREAD, s.passphrase);
      await (await shown(b, '[title="Threads"]')).click();
      await byText(b, ".thread-list-item-name", THREAD, NET_WAIT_MS);
      await shot();
    },
  },
  {
    id: "M76",
    title: "Thread list: open a thread, M77 send",
    async run({ b, s, shot }) {
      await (await byText(b, ".thread-list-item", THREAD)).click();
      await byText(b, ".thread-panel-title", THREAD, NET_WAIT_MS);
      await typeInto(await shown(b, ".thread-panel .message-input-field"), "thread reply");
      await b.keys("Enter");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await byText(b, ".thread-panel-messages", "thread reply", NET_WAIT_MS);
      await shot();
    },
  },
  {
    id: "M78",
    title: "Thread: archive, M79 close",
    async run({ b, s, shot }) {
      await (await shown(b, '[aria-label="Archive thread"]')).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await shown(b, 'textarea[placeholder="Reply to reactivate thread..."], input[placeholder="Reply to reactivate thread..."]', NET_WAIT_MS);
      await shot();
      await (await b.$('[aria-label="Close thread"]')).click();
      await gone(b, ".thread-panel");
    },
  },
  {
    id: "M53",
    title: "Create poll",
    async run({ b, s, shot }) {
      await msgAction(b, EDITED, "Create poll");
      await typeInto(await shown(b, 'input[placeholder="Poll question..."]'), "walk poll?");
      await typeInto(await shown(b, 'input[placeholder="Answer 1"]'), "yes");
      await typeInto(await shown(b, 'input[placeholder="Answer 2"]'), "no");
      await (await b.$('.modal-container button[type="submit"]')).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await byText(b, ".poll-card-question", "walk poll?", NET_WAIT_MS);
      await shot();
    },
  },
  {
    id: "M47",
    title: "Delete message",
    async run({ b, s, shot }) {
      await msgAction(b, EDITED, "Delete message");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await waitUntil(
        async () => {
          for (const m of await b.$$(".community-main .chat-message")) if ((await m.getText()).includes(EDITED)) return false;
          return true;
        },
        NET_WAIT_MS,
        "message still shown",
      );
      await shot();
    },
  },
  {
    id: "M22",
    title: "Search (button)",
    async run({ b, shot }) {
      await (await shown(b, '[aria-label="Search community"]')).click();
      await typeInto(await shown(b, '.search-panel input[placeholder="Search…"]'), "hello");
      await b.keys("Enter");
      await sleep(2000);
      await shot();
      const text = (await (await b.$(".search-panel")).getText()).replace(/\s+/g, " ").slice(0, 120);
      await (await b.$(".search-panel-close")).click();
      await gone(b, ".search-panel");
      return `panel: ${text}`;
    },
  },
  {
    id: "M19",
    title: "Create category",
    async run({ b, s, shot }) {
      await (await shown(b, '[title="Create Category"]')).click();
      await modalSubmit(b, 'input[placeholder="Category name"]', CATEGORY, s.passphrase);
      await byText(b, ".channel-list", CATEGORY, NET_WAIT_MS);
      await shot();
    },
  },
  {
    id: "M13",
    title: "Channel menu > Rename",
    async run({ b, s, shot }) {
      await contextMenu(b, await channel(b, TEXT_CH), "Rename");
      await modalSubmit(b, '.modal-container input[placeholder="Channel name..."]', RENAMED_CH, s.passphrase);
      await channel(b, RENAMED_CH);
      await shot();
    },
  },
  {
    id: "M14",
    title: "Channel menu > Delete",
    async run({ b, s, shot }) {
      await contextMenu(b, await channel(b, "walk-voice"), "Delete");
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await waitUntil(async () => !(await (await b.$(".channel-list")).getText()).includes("walk-voice"), NET_WAIT_MS, "walk-voice still listed");
      await shot();
    },
  },
  {
    id: "M33",
    title: "Events: create",
    async run({ b, shot }) {
      await shot();
      if (!(await present(b, ".sidebar-events-section .sidebar-section-more"))) {
        throw new Error("no control opens the Events panel: CommunitySidebar.tsx:134 renders the section only when an upcoming event exists");
      }
    },
  },
  {
    id: "M39",
    title: "Game servers: add",
    async run({ b, shot }) {
      await shot();
      if (!(await present(b, ".sidebar-servers-section .sidebar-section-more"))) {
        throw new Error("no control opens the game server list: CommunitySidebar.tsx:97,121 show it only with more than 3 servers");
      }
    },
  },
  {
    id: "CS1",
    title: "Community settings: Overview save description",
    async run({ b, s, shot }) {
      await openCommunitySettings(b, "Overview");
      await typeInto(await shown(b, 'textarea[placeholder="Community description..."]'), "walk description");
      await (await byText(b, ".modal-container .form-btn-primary", "Save")).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await closeCommunityModal(b);
      await openCommunitySettings(b, "Overview");
      // An admin edits the description in place: the reopened tab fills the
      // textarea from the stored community (OverviewTab.tsx:19-22,68-75);
      // `.settings-value` is the read-only fallback for non-admins.
      const desc = 'textarea[placeholder="Community description..."]';
      await waitUntil(async () => (await (await shown(b, desc)).getValue()) === "walk description", NET_WAIT_MS, "reopened Overview does not show the saved description");
      await shot();
    },
  },
  {
    id: "CS28",
    title: "Roles: create",
    async run({ b, s, shot }) {
      await openCommunitySettings(b, "Roles");
      await (await byText(b, ".modal-container .form-btn-secondary", "Create Role")).click();
      await typeInto(await shown(b, 'input[placeholder="Role name..."]'), "walk-role");
      await (await byText(b, ".modal-container .form-btn-primary", "Create")).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await byText(b, ".settings-role-row", "walk-role", NET_WAIT_MS);
      await shot();
    },
  },
  {
    id: "CS25",
    title: "Invites: create",
    async run({ b, s, shot }) {
      await openCommunitySettings(b, "Invites");
      await (await shown(b, ".modal-container .settings-content .form-btn-primary")).click();
      const create = await byText(b, ".modal-container .form-btn-primary", "Create");
      await create.click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await shown(b, ".invite-link-display", NET_WAIT_MS);
      await shot();
    },
  },
  {
    id: "CS32",
    title: "AutoMod: save rule, CS33 delete",
    async run({ b, s, shot }) {
      await openCommunitySettings(b, "AutoMod");
      await typeInto(await shown(b, 'input[placeholder="Blocked words"]'), "walk-rule");
      await typeInto(await shown(b, 'textarea[placeholder="One keyword per line"]'), "badword");
      // A new rule's button reads "Create Rule" ("Save Rule" only while editing; AutoModTab.tsx:140-141).
      await (await byText(b, ".modal-container .form-btn-primary", "Create Rule")).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await byText(b, ".automod-rule-list", "walk-rule", NET_WAIT_MS);
      await shot();
      await (await shown(b, ".automod-rule-delete-btn")).click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 3000 });
      await waitUntil(async () => !(await present(b, ".automod-rule-delete-btn")), NET_WAIT_MS, "rule still listed after Delete");
      return "rule listed, then deleted";
    },
  },
  {
    id: "CS36",
    title: "Audit log: view",
    async run({ b, shot }) {
      // An admin finds the log folded into Security (CommunitySettingsModal.tsx:89-90,166-172).
      await openCommunitySettings(b, "Security");
      await (await shown(b, ".settings-audit-collapsible summary")).click();
      // One row per entry (AuditLogTab.tsx:50): webdriverio's strict `$`
      // refuses a selector with several matches, so the rows are counted.
      let n = 0;
      await waitUntil(
        async () => {
          const rows = await b.$$(".audit-log-entry");
          n = rows.length;
          return n > 0 && (await rows[0].isDisplayed());
        },
        NET_WAIT_MS,
        "no audit log entry shown",
      );
      await shot();
      await closeCommunityModal(b);
      if (n === 0) throw new Skip("no entries");
      return `${n} entries`;
    },
  },
];
