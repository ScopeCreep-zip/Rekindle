import { isCommandError } from "../ipc/commands/dto";
import { addToast } from "../stores/toast.store";
import { errorMessage } from "./error";

/// Each conversation's last queued draft report: the next waits on it.
const chains = new Map<string, Promise<void>>();

/// When a draft report's failure was last said (ms): a report goes per
/// composer edit, so one failure would otherwise toast on every key.
let failureShownAt = 0;
const FAILURE_EVERY_MS = 30_000;

/**
 * Send one composer draft report (whether the box holds text) for the
 * conversation `key`, after the reports queued before it (plan E3.4 S9):
 * the Host decides what typing to send from the last report it reads, so
 * reports must reach it in edit order. Sent concurrently, a "cleared" from
 * an earlier edit could land after the "typing" of a later one and end the
 * draft (r24 channel-typing on Linux: false 330 ms after true, a field
 * cleared then typed into). A refusal is said, at most once per 30 s, never
 * swallowed; a cancelled review says nothing.
 */
export function sendDraftReport(key: string, send: () => Promise<unknown>): Promise<void> {
  const next = (chains.get(key) ?? Promise.resolve()).then(async () => {
    try {
      await send();
    } catch (e) {
      if (isCommandError(e) && e.type === "cancelled") return;
      const now = Date.now();
      if (now - failureShownAt < FAILURE_EVERY_MS) return;
      failureShownAt = now;
      addToast(`Typing not sent: ${errorMessage(e)}`, "error");
    }
  });
  chains.set(key, next);
  // The chain ends with its last report, so a quiet conversation holds no promise.
  void next.then(() => {
    if (chains.get(key) === next) chains.delete(key);
  });
  return next;
}
