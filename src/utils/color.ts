/**
 * Role colour conversion between the packed-int wire form and CSS hex.
 *
 * `MemberProfilePopup` carried its own `formatColorHex`/`parseColorHex`
 * with the same bit-masking core but two real differences, both kept
 * here rather than flattened away:
 *
 *  * a different fallback for a missing colour (its accent blue, not
 *    black) — so that is now a parameter, not a hardcoded constant;
 *  * a parse that validates length and finiteness instead of letting
 *    `parseInt` return `NaN` — that is `parseColorHex` below.
 */

/** Default colour when a role has none set. */
export const DEFAULT_ROLE_COLOR = "#000000";

/**
 * Render a packed 24-bit colour as `#rrggbb`.
 *
 * `fallback` is returned for a null/undefined/zero colour — callers
 * differ on what "no colour" should look like.
 */
export function colorIntToHex(n: number | null | undefined, fallback = DEFAULT_ROLE_COLOR): string {
  if (!n) return fallback;
  return `#${(n & 0xffffff).toString(16).padStart(6, "0")}`;
}

/**
 * Parse `#rrggbb` (or bare `rrggbb`) into a packed 24-bit int.
 *
 * Returns `null` for anything that is not exactly six hex digits,
 * rather than the `NaN` a bare `parseInt` would produce.
 */
export function parseColorHex(input: string): number | null {
  const trimmed = input.trim().replace(/^#/, "");
  if (trimmed.length !== 6) return null;
  const parsed = Number.parseInt(trimmed, 16);
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * Lenient parse kept for existing callers: accepts any length and can
 * return `NaN`. Prefer [`parseColorHex`].
 */
export function hexToColorInt(hex: string): number {
  return parseInt(hex.replace("#", ""), 16);
}
