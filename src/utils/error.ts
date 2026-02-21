/** Extract a human-readable message from an unknown catch value. */
export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
