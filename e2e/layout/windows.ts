import { expect, type Page } from "@playwright/test";
import { preloadMocks } from "../fixtures/mocks";
import { CALL, CHANNEL, COMMUNITY, GROUP, PEER, stressHandler, type Stress } from "../fixtures/stress";

// The routed windows (`src/main.tsx` Switch) at the sizes the shell opens
// them (`src-tauri/src/windows/mod.rs` `size()`: default, minimum). The
// large size is half again the default, inside any maximum the shell sets
// (the buddy list's `max_inner_size(400, 900)`).

export interface Size {
  width: number;
  height: number;
}

export interface WindowCase {
  name: string;
  label: string;
  path: string;
  min: Size;
  def: Size;
  max?: Size;
  /** Run after `goto`, until the window shows its content. */
  ready: (page: Page) => Promise<void>;
  /** Containers whose declared gaps must sit on the spacing grid. */
  spacing: string[];
  /** More such containers, rendered only when the window has data. */
  spacingWithData?: string[];
  /** Content panes, rendered when the window has data, that must never scroll sideways. */
  panes?: string[];
}

const frame = (page: Page) => expect(page.locator(".app-frame").first()).toBeVisible();

export const WINDOWS: WindowCase[] = [
  {
    name: "login",
    label: "login",
    path: "/login",
    def: { width: 380, height: 480 },
    min: { width: 340, height: 440 },
    ready: frame,
    spacing: [".xfire-titlebar"],
  },
  {
    name: "buddy-list",
    label: "buddy-list",
    path: "/buddy-list",
    def: { width: 320, height: 650 },
    min: { width: 300, height: 500 },
    max: { width: 400, height: 900 },
    ready: frame,
    spacing: [".xfire-titlebar", ".status-bar", ".menu-bar"],
    spacingWithData: [".buddy-group-header"],
    panes: [".buddy-list"],
  },
  {
    name: "chat-dm",
    label: `chat-${PEER.slice(0, 16)}`,
    path: `/chat?peer=${PEER}`,
    def: { width: 480, height: 550 },
    min: { width: 380, height: 400 },
    ready: frame,
    spacing: [".xfire-titlebar", ".message-input-wrapper", ".message-input-shell"],
    panes: [".chat-message-area"],
  },
  {
    name: "chat-group",
    label: `chat-g${GROUP}`,
    path: `/chat?group=${GROUP}`,
    def: { width: 480, height: 550 },
    min: { width: 380, height: 400 },
    ready: frame,
    spacing: [".xfire-titlebar", ".group-chat-header", ".group-chat-members", ".message-input-wrapper"],
    panes: [".chat-message-area"],
  },
  {
    name: "community",
    label: "community",
    path: `/community?id=${COMMUNITY}`,
    def: { width: 900, height: 650 },
    min: { width: 750, height: 500 },
    ready: frame,
    spacing: [".xfire-titlebar", ".community-sidebar-header"],
    spacingWithData: [".channel-list", ".member-list", ".member-list-header", ".community-channel-header"],
    panes: [".chat-message-area", ".channel-list", ".member-list"],
  },
  {
    name: "call",
    label: `call-${CALL}`,
    path: `/call?id=${CALL}`,
    def: { width: 420, height: 540 },
    min: { width: 360, height: 420 },
    ready: frame,
    spacing: [".xfire-titlebar", ".active-call-controls"],
  },
  {
    name: "profile",
    label: `profile-${PEER.slice(0, 16)}`,
    path: `/profile?key=${PEER}`,
    def: { width: 380, height: 500 },
    min: { width: 340, height: 420 },
    ready: frame,
    spacing: [".xfire-titlebar"],
  },
  {
    name: "settings",
    label: "settings",
    path: "/settings",
    def: { width: 500, height: 550 },
    min: { width: 420, height: 450 },
    ready: frame,
    spacing: [".xfire-titlebar", ".form-tabs"],
  },
];

/** The content panes `w` renders with or without data. */
export function panesOf(w: WindowCase, withData: boolean): string[] {
  return withData ? w.panes ?? [] : [];
}

/** The spacing containers `w` renders with or without data. */
export function spacingOf(w: WindowCase, withData: boolean): string[] {
  return withData ? [...w.spacing, ...(w.spacingWithData ?? [])] : w.spacing;
}

export function sizes(w: WindowCase): [string, Size][] {
  const large = {
    width: Math.round(w.def.width * 1.5),
    height: Math.round(w.def.height * 1.5),
  };
  if (w.max) {
    large.width = Math.min(large.width, w.max.width);
    large.height = Math.min(large.height, w.max.height);
  }
  return [["min", w.min], ["default", w.def], ["large", large]];
}

/** Open `w` at `size` over the stress data and wait for it to settle. */
export async function open(
  page: Page,
  w: WindowCase,
  size: Size,
  data: Stress,
  base: "dark" | "light" = "dark",
): Promise<void> {
  await page.setViewportSize(size);
  await preloadMocks(page, w.label, stressHandler(data, base));
  await page.goto(w.path);
  await w.ready(page);
  // Lazy windows hydrate over several IPC rounds; let them land.
  await page.waitForLoadState("networkidle");
  await page.waitForTimeout(300);
}

export { CHANNEL };
