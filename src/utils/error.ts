/** Extract a human-readable message from an unknown catch value: an
 *  `Error`, a Host call's `CommandError` (its message and what to do), or
 *  anything else as text. */
export function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "object" && e !== null && "type" in e && "data" in e) {
    const { type, data } = e as { type: string; data: Record<string, unknown> };
    if (type === "failed" && typeof data.message === "string") {
      return typeof data.remediation === "string"
        ? `${data.message} — ${data.remediation}`
        : data.message;
    }
    if (type === "intentRequired") return "This action needs your review.";
  }
  return String(e);
}
