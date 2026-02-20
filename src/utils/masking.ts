/**
 * Mask a long string for display, keeping a prefix and suffix visible.
 *
 * Example: maskString("rekindle://abcdefghijklmnop", "rekindle://", 8, 6)
 *          → "rekindle://abcdefgh...klmnop"
 */
export function maskString(
  value: string,
  prefix: string,
  keepStart: number,
  keepEnd: number,
): string {
  if (!value.startsWith(prefix)) return value;
  const body = value.slice(prefix.length);
  if (body.length <= keepStart + keepEnd) return value;
  return `${prefix}${body.slice(0, keepStart)}...${body.slice(-keepEnd)}`;
}

/**
 * Mask a `rekindle://` invite URL for display.
 *
 * Shows the first 8 and last 6 characters of the encoded portion.
 * Example: "rekindle://abcdefgh...uvwxyz"
 */
export function maskInviteUrl(url: string): string {
  return maskString(url, "rekindle://", 8, 6);
}
