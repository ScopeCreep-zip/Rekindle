// Plan E7.14 S5: press a key as native input through the driver's Rekindle
// extension (`POST /session/{id}/rekindle/key`, vendor handlers/native_key.rs),
// so the engine performs the key's default action (Tab moves focus) and the
// page sees a trusted event; webdriverio's `keys()` sends synthetic events,
// which never move focus. Linux only: GTK simulates the key on the WebView
// widget. macOS answers "unsupported operation": its session window cannot
// be found without new unsafe code (objc2-web-kit 0.3.2 WKWebView::URL and
// title are unsafe fn), so the rows that need a real Tab are owed manual
// checks there.

import type { Browser } from "webdriverio";

/** `"sent"`, or the driver's reason it cannot send native keys here. */
export async function nativeKey(b: Browser, key: string, shift = false): Promise<"sent" | { unsupported: string }> {
  const { protocol = "http", hostname = "127.0.0.1", port, headers = {} } = b.options;
  const response = await fetch(`${protocol}://${hostname}:${port}/session/${b.sessionId}/rekindle/key`, {
    method: "POST",
    headers: { ...headers, "Content-Type": "application/json" },
    body: JSON.stringify({ key, shift }),
  });
  if (response.ok) return "sent";
  const body = (await response.json().catch(() => null)) as { value?: { error?: string; message?: string } } | null;
  if (body?.value?.error === "unsupported operation") return { unsupported: body.value.message ?? "unsupported operation" };
  throw new Error(`native key ${key} failed: HTTP ${response.status} ${body?.value?.error ?? ""} ${body?.value?.message ?? ""}`);
}
