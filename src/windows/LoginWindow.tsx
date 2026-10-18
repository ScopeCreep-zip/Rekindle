import { Component, createSignal, onMount, onCleanup, For, Show } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";
import Avatar from "../components/common/Avatar";
import LoadingButton from "../components/common/LoadingButton";
import { handleLogin, handleCreateIdentity } from "../actions/auth.actions";
import { commands, IdentitySummary } from "../ipc/commands";
import { isCommandError } from "../ipc/commands/dto";
import { canUnlock, setLifecycleState } from "../stores/lifecycle.store";
import { subscribeLifecycleEvents } from "../ipc/channels/subscriptions";
import { errorMessage } from "../utils/error";
import { truncateKey } from "../utils/formatting";
import { startEventStream } from "../ipc/channels";

type Mode = "picker" | "login" | "create";

const LoginWindow: Component = () => {
  const [identities, setIdentities] = createSignal<IdentitySummary[]>([]);
  const [mode, setMode] = createSignal<Mode>("create");
  const [selected, setSelected] = createSignal<IdentitySummary | null>(null);
  const [passphrase, setPassphrase] = createSignal("");
  const [displayName, setDisplayName] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);

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

  onMount(async () => {
    loadIdentities();
    // Seed the derived lifecycle view from the single backend authority,
    // then observe transitions — mirrors Briar's StartupViewModel. The
    // login button stays "Connecting…" until the node attaches (locked).
    try {
      setLifecycleState(await commands.lifecycleCurrent());
    } catch {
      // lifecycle_current unavailable (very early boot) — the subscription
      // below resyncs on the first transition.
    }
    const unlisten = await subscribeLifecycleEvents(setLifecycleState);
    void startEventStream();
    onCleanup(unlisten);
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
    if (!sel || !passphrase().trim() || loading() || !canUnlock()) return;

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
    if (!passphrase().trim() || loading() || !canUnlock()) return;

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

  /** Delete an identity: Protected, so the Host's review asks for its
   *  passphrase in the `IntentDialog` — the one prompt (ADR 0019). */
  async function deleteIdentity(id: IdentitySummary, e: Event): Promise<void> {
    e.stopPropagation();
    setError(null);
    try {
      await commands.deleteIdentity(id.publicKey);
      await loadIdentities();
    } catch (err) {
      // A cancelled review is the user's choice, not a failure.
      if (isCommandError(err) && err.type === "failed" && err.data.code === 499) return;
      setError(errorMessage(err));
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
                  <Avatar displayName={id.displayName} size={64} />
                  <div class="account-bubble-name">{id.displayName}</div>
                  <button
                    class="account-bubble-delete"
                    aria-label={`Delete ${id.displayName}`}
                    onClick={(e: MouseEvent) => void deleteIdentity(id, e)}
                  >
                    ✕
                  </button>
                </div>
              )}
            </For>
          </div>
          <Show when={error() !== null}>
            <div class="form-error" role="alert">{error()}</div>
          </Show>
          <button class="account-create-btn" onClick={() => { setPassphrase(""); setDisplayName(""); setError(null); setMode("create"); }}>
            + Create New Identity
          </button>
        </div>
      </Show>

      {/* Login / lock screen mode */}
      <Show when={mode() === "login" && selected() !== null}>
        <form class="login-container" onSubmit={handleLoginSubmit}>
          <div class="lock-avatar">
            <Avatar displayName={selected()!.displayName} size={96} />
          </div>
          <div class="lock-name">{selected()!.displayName}</div>
          <div class="account-card-key">{truncateKey(selected()!.publicKey)}</div>
          <input
            class="form-input"
            type="password"
            placeholder="Passphrase"
            value={passphrase()}
            onInput={(e: InputEvent) => setPassphrase((e.target as HTMLInputElement).value)}
            autofocus
          />
          <Show when={error() !== null}>
            <div class="form-error">{error()}</div>
          </Show>
          <LoadingButton
            type="submit"
            loading={loading()}
            disabled={!canUnlock()}
            loadingLabel="Unlocking"
          >
            {canUnlock() ? "Unlock" : "Connecting…"}
          </LoadingButton>
          <button type="button" class="form-btn-secondary" onClick={goBack}>
            ← Switch Account
          </button>
        </form>
      </Show>

      {/* Create mode */}
      <Show when={mode() === "create"}>
        <form class="login-container" onSubmit={handleCreateSubmit}>
          <Show when={identities().length > 0}>
            <button type="button" class="form-btn-secondary" onClick={goBack}>
              ← Back
            </button>
          </Show>
          <div class="login-title">Rekindle</div>
          <div class="login-subtitle">Create a passphrase for your new identity</div>
          <input
            class="form-input"
            type="text"
            placeholder="Display Name (optional)"
            value={displayName()}
            onInput={(e: InputEvent) => setDisplayName((e.target as HTMLInputElement).value)}
          />
          <input
            class="form-input"
            type="password"
            placeholder="Passphrase"
            value={passphrase()}
            onInput={(e: InputEvent) => setPassphrase((e.target as HTMLInputElement).value)}
            autofocus
          />
          <Show when={error() !== null}>
            <div class="form-error">{error()}</div>
          </Show>
          <LoadingButton
            type="submit"
            loading={loading()}
            disabled={!canUnlock()}
            loadingLabel="Creating identity"
          >
            {canUnlock() ? "Create Identity" : "Connecting…"}
          </LoadingButton>
        </form>
      </Show>

    </div>
  );
};

export default LoginWindow;
