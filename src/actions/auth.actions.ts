import { commands } from "../ipc/commands";
import { authState, setAuthState } from "../stores/auth.store";
import { addToast } from "../stores/toast.store";
import { fetchAvatarUrl } from "../ipc/avatar";
import { errorMessage } from "../utils/error";

export async function handleLogin(
  publicKey: string,
  passphrase: string,
): Promise<{ success: true } | { success: false; error: string }> {
  try {
    const result = await commands.login(publicKey, passphrase);
    const avatarUrl = await fetchAvatarUrl(result.publicKey);
    setAuthState({
      isLoggedIn: true,
      publicKey: result.publicKey,
      fingerprint: result.fingerprint,
      displayName: result.displayName,
      avatarUrl,
      status: "online",
    });
    return { success: true };
  } catch (e) {
    const message = errorMessage(e);
    console.error("Login failed:", message);
    return { success: false, error: message };
  }
}

export async function handleCreateIdentity(
  passphrase: string,
  displayName?: string,
): Promise<{ success: true } | { success: false; error: string }> {
  try {
    const result = await commands.createIdentity(passphrase, displayName);
    setAuthState({
      isLoggedIn: true,
      publicKey: result.publicKey,
      fingerprint: result.fingerprint,
      displayName: result.displayName,
      avatarUrl: null,
      status: "online",
    });
    return { success: true };
  } catch (e) {
    const message = errorMessage(e);
    console.error("Create identity failed:", message);
    return { success: false, error: message };
  }
}

export async function handleLogout(): Promise<void> {
  try {
    await commands.logout();
    setAuthState({
      isLoggedIn: false,
      publicKey: null,
      fingerprint: null,
      displayName: null,
      avatarUrl: null,
      status: "offline",
    });
  } catch (e) {
    console.error("Logout failed:", e);
  }
}

/** Set our display name (the name field, `UserIdentityBar` and the skin
 *  tree's `NameField`): shown at once, reverted with the Host's reason on
 *  a refusal (walk 23 B12: a refusal was only logged). */
export async function handleSetDisplayName(raw: string): Promise<void> {
  const name = raw.trim();
  if (!name || name === authState.displayName) return;
  const oldName = authState.displayName;
  setAuthState("displayName", name);
  try {
    await commands.setNickname(name);
  } catch (e) {
    setAuthState("displayName", oldName);
    addToast(`Name not changed: ${errorMessage(e)}`, "error");
  }
}

/** Set our status message (Xfire's status field): shown at once, reverted
 *  on a refusal. */
export async function handleSetStatusMessage(raw: string): Promise<void> {
  const msg = raw.trim();
  const oldMsg = authState.statusMessage;
  if (msg === (oldMsg ?? "")) return;
  setAuthState("statusMessage", msg || null);
  try {
    await commands.setStatusMessage(msg);
  } catch (e) {
    console.error("Failed to set status message:", e);
    setAuthState("statusMessage", oldMsg);
  }
}
