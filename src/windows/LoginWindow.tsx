import { Component, createSignal, onMount, For, Show } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";
import Avatar from "../components/common/Avatar";
import Modal from "../components/common/Modal";
import { handleLogin, handleCreateIdentity } from "../handlers/auth.handlers";
import { commands, avatarDataUrl, IdentitySummary } from "../ipc/commands";
import { errorMessage } from "../utils/error";

type Mode = "picker" | "login" | "create";

function truncateKey(key: string): string {
  if (key.length <= 16) return key;
  return `${key.slice(0, 8)}...${key.slice(-8)}`;
}

const LoginWindow: Component = () => {
  const [identities, setIdentities] = createSignal<IdentitySummary[]>([]);
  const [mode, setMode] = createSignal<Mode>("create");
  const [selected, setSelected] = createSignal<IdentitySummary | null>(null);
  const [passphrase, setPassphrase] = createSignal("");
  const [displayName, setDisplayName] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);

  // Delete confirmation state
  const [deleteTarget, setDeleteTarget] = createSignal<IdentitySummary | null>(null);
  const [deletePass, setDeletePass] = createSignal("");
  const [deleteError, setDeleteError] = createSignal<string | null>(null);

  async function loadIdentities(): Promise<void> {
    try {
      const list = await commands.listIdentities();
      setIdentities(list);

      // If an ?account= query param is present (e.g. returning from logout),
      // jump directly to the passphrase screen for that account.
      const params = new URLSearchParams(window.location.search);
      const preselect = params.get("account");
      if (preselect) {
        const match = list.find((id) => id.publicKey === preselect);
        if (match) {
          selectAccount(match);
          return;
        }
      }

      if (list.length > 0) {
        setMode("picker");
      } else {
        setMode("create");
      }
    } catch {
      setMode("create");
    }
  }

  onMount(() => {
    loadIdentities();
  });

  function selectAccount(id: IdentitySummary): void {
    setSelected(id);
    setPassphrase("");
    setError(null);
    setMode("login");
  }

  function goBack(): void {
    setPassphrase("");
    setError(null);
    if (identities().length > 0) {
      setMode("picker");
    } else {
      setMode("create");
    }
  }

  async function handleLoginSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const sel = selected();
    if (!sel || !passphrase().trim() || loading()) return;

    setLoading(true);
    setError(null);

    const result = await handleLogin(sel.publicKey, passphrase());

    if (result.success) {
      try {
        await commands.showBuddyList();
      } catch (err) {
        const msg = errorMessage(err);
        console.error("Failed to open buddy list:", msg);
        setError(msg);
        setLoading(false);
      }
    } else {
      setError(result.error);
      setLoading(false);
    }
  }

  async function handleCreateSubmit(e: Event): Promise<void> {
    e.preventDefault();
    if (!passphrase().trim() || loading()) return;

    setLoading(true);
    setError(null);

    const result = await handleCreateIdentity(passphrase(), displayName() || undefined);

    if (result.success) {
      try {
        await commands.showBuddyList();
      } catch (err) {
        const msg = errorMessage(err);
        console.error("Failed to open buddy list:", msg);
        setError(msg);
        setLoading(false);
      }
    } else {
      setError(result.error);
      setLoading(false);
    }
  }

  function confirmDelete(id: IdentitySummary, e: Event): void {
    e.stopPropagation();
    setDeleteTarget(id);
    setDeletePass("");
    setDeleteError(null);
  }

  function cancelDelete(): void {
    setDeleteTarget(null);
    setDeletePass("");
    setDeleteError(null);
  }

  async function executeDelete(): Promise<void> {
    const target = deleteTarget();
    if (!target || !deletePass().trim()) return;

    try {
      await commands.deleteIdentity(target.publicKey, deletePass());
      cancelDelete();
      await loadIdentities();
    } catch (err) {
      setDeleteError(errorMessage(err));
    }
  }

  return (
    <div class="app-frame">
      <Titlebar title="Rekindle" />

      {/* Picker mode — avatar bubble grid */}
      <Show when={mode() === "picker"}>
        <div class="account-picker">
          <div class="login-title">Rekindle</div>
          <div class="account-picker-subtitle">Select an account</div>
          <div class="account-bubble-grid">
            <For each={identities()}>
              {(id) => (
                <div class="account-bubble" onClick={() => selectAccount(id)}>
                  <Avatar
                    displayName={id.displayName}
                    size={64}
                    avatarUrl={avatarDataUrl(id.avatarBase64)}
                  />
                  <div class="account-bubble-name">{id.displayName}</div>
                  <button
                    class="account-bubble-delete"
                    onClick={(e: MouseEvent) => confirmDelete(id, e)}
                  >
                    ✕
                  </button>
                </div>
              )}
            </For>
          </div>
          <button class="account-create-btn" onClick={() => { setPassphrase(""); setDisplayName(""); setError(null); setMode("create"); }}>
            + Create New Identity
          </button>
        </div>
      </Show>

      {/* Login / lock screen mode */}
      <Show when={mode() === "login" && selected() !== null}>
        <form class="login-container" onSubmit={handleLoginSubmit}>
          <div class="lock-avatar">
            <Avatar
              displayName={selected()!.displayName}
              size={96}
              avatarUrl={avatarDataUrl(selected()!.avatarBase64)}
            />
          </div>
          <div class="lock-name">{selected()!.displayName}</div>
          <div class="account-card-key">{truncateKey(selected()!.publicKey)}</div>
          <input
            class="login-input"
            type="password"
            placeholder="Passphrase"
            value={passphrase()}
            onInput={(e: InputEvent) => setPassphrase((e.target as HTMLInputElement).value)}
            autofocus
          />
          <Show when={error() !== null}>
            <div class="login-error">{error()}</div>
          </Show>
          <button class="login-btn" type="submit" disabled={loading()}>
            {loading() ? "..." : "Unlock"}
          </button>
          <button type="button" class="account-back-btn" onClick={goBack}>
            ← Switch Account
          </button>
        </form>
      </Show>

      {/* Create mode */}
      <Show when={mode() === "create"}>
        <form class="login-container" onSubmit={handleCreateSubmit}>
          <Show when={identities().length > 0}>
            <button type="button" class="account-back-btn" onClick={goBack}>
              ← Back
            </button>
          </Show>
          <div class="login-title">Rekindle</div>
          <div class="login-subtitle">Create a passphrase for your new identity</div>
          <input
            class="login-input"
            type="text"
            placeholder="Display Name (optional)"
            value={displayName()}
            onInput={(e: InputEvent) => setDisplayName((e.target as HTMLInputElement).value)}
          />
          <input
            class="login-input"
            type="password"
            placeholder="Passphrase"
            value={passphrase()}
            onInput={(e: InputEvent) => setPassphrase((e.target as HTMLInputElement).value)}
            autofocus
          />
          <Show when={error() !== null}>
            <div class="login-error">{error()}</div>
          </Show>
          <button class="login-btn" type="submit" disabled={loading()}>
            {loading() ? "..." : "Create Identity"}
          </button>
        </form>
      </Show>

      {/* Delete confirmation modal */}
      <Modal
        isOpen={deleteTarget() !== null}
        title="Delete Account"
        onClose={cancelDelete}
      >
        <div class="delete-confirm-text">
          Enter passphrase for "{deleteTarget()?.displayName}" to confirm deletion.
        </div>
        <input
          class="login-input"
          type="password"
          placeholder="Passphrase"
          value={deletePass()}
          onInput={(e: InputEvent) => setDeletePass((e.target as HTMLInputElement).value)}
        />
        <Show when={deleteError() !== null}>
          <div class="login-error">{deleteError()}</div>
        </Show>
        <button
          class="delete-confirm-btn"
          onClick={executeDelete}
          disabled={!deletePass().trim()}
        >
          Delete Forever
        </button>
        <button class="delete-confirm-cancel" onClick={cancelDelete}>
          Cancel
        </button>
      </Modal>
    </div>
  );
};

export default LoginWindow;
