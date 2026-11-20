import { commands } from "../ipc/commands";
import { setAuthState } from "../stores/auth.store";
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
      displayName: null,
      avatarUrl: null,
      status: "offline",
    });
  } catch (e) {
    console.error("Logout failed:", e);
  }
}
