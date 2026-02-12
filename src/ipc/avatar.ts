import { commands } from "./commands";

/**
 * Fetch a user's avatar from the backend and return a data URL.
 *
 * Returns `null` if no avatar is set or the fetch fails.
 */
export async function fetchAvatarUrl(publicKey: string): Promise<string | null> {
  try {
    const bytes = await commands.getAvatar(publicKey);
    if (!bytes || bytes.length === 0) return null;
    const uint8 = new Uint8Array(bytes);
    let binary = "";
    for (let i = 0; i < uint8.length; i++) {
      binary += String.fromCharCode(uint8[i]);
    }
    const base64 = btoa(binary);
    return `data:image/webp;base64,${base64}`;
  } catch {
    return null;
  }
}
