import { type Component, createSignal, createResource, For, Show } from "solid-js";
import Avatar from "../../components/common/Avatar";
import FormField from "../../components/common/FormField";
import { authState, setAuthState } from "../../stores/auth.store";
import { handleSetAvatar } from "../../actions/settings.actions";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import { errorMessage } from "../../utils/error";
import { fetchAvatarUrl } from "../../ipc/avatar";
import GameIcon from "../../components/game/GameIcon";
import { formatDuration } from "../../utils/formatting";
import { formatRelativeTime } from "../../utils/time";
import { useDraft } from "../../hooks/useDraft";

/// The Host's cap (`rekindle_types::presence::limits::MAX_BIO_LEN`,
/// Discord's About Me); the Host refuses a longer one.
const MAX_BIO_LEN = 190;

const ProfileTab: Component = () => {
  // The name shows the hydrated identity until the user edits it; the
  // window's first `hydrateState` resolves after the tab mounts, so a
  // store that fills in late must never overwrite what they typed (walk
  // row ST3: the typed name was replaced by the old one before Enter).
  const name = useDraft(() => authState.displayName ?? "");
  const [statusMsgInput, setStatusMsgInput] = createSignal("");

  // Our time in each detected game and when we last played it (plan E7.2),
  // from the Host's `game_sessions`.
  const [gameTotals] = createResource(() => commands.getGameTotals());

  // Our bio (plan E7.2), from `identity.bio`; it reaches friends only as a
  // sealed message, never the profile record.
  // The same rule as the name: `getBio` may answer after the user began.
  const [storedBio, { mutate: setStoredBio }] = createResource(() => commands.getBio());
  const bio = useDraft(() => storedBio() ?? "");
  // Characters as the Host counts them (Unicode scalar values).
  const bioLength = () => [...bio.value()].length;

  async function handleSaveBio(): Promise<void> {
    if (bioLength() > MAX_BIO_LEN) return;
    try {
      setStoredBio(await commands.setBio(bio.value()));
      bio.settle();
    } catch (e) {
      console.error("Failed to set bio:", e);
    }
  }

  /// Save `typed`, the field's text as it is now (Enter passes the input's
  /// own value). The Host reviews it (U1); until it commits, the field
  /// keeps the edit, and a refused or cancelled change leaves it there.
  function handleSaveName(typed: string): void {
    const next = typed.trim();
    if (next && next !== authState.displayName) {
      const oldName = authState.displayName;
      setAuthState("displayName", next);
      commands.setNickname(next).then(
        () => name.settle(),
        (e: unknown) => {
          setAuthState("displayName", oldName);
          addToast(`Name not changed: ${errorMessage(e)}`, "error");
        },
      );
    }
  }

  function handleSaveStatusMessage(): void {
    const msg = statusMsgInput().trim();
    commands.setStatusMessage(msg).catch((e) => {
      console.error("Failed to set status message:", e);
    });
  }

  async function handleAvatarUpload(): Promise<void> {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png, image/jpeg, image/gif";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const arrayBuffer = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(arrayBuffer));
      // The picker's MIME type declares the kind; the Host decodes it as
      // that kind only. An unsupported file is shown, not uploaded.
      const set = await handleSetAvatar(bytes, file.type);
      // Refresh avatar in store after upload
      if (set && authState.publicKey) {
        const avatarUrl = await fetchAvatarUrl(authState.publicKey);
        setAuthState("avatarUrl", avatarUrl);
      }
    };
    input.click();
  }

  return (
    <>
      <div class="settings-section-title">Avatar</div>
      <div class="avatar-upload-section">
        <Avatar displayName={authState.displayName ?? "?"} size={64} avatarUrl={authState.avatarUrl ?? undefined} />
        <button type="button" class="avatar-upload-btn" onClick={handleAvatarUpload}>
          Change Avatar
        </button>
        <span class="avatar-upload-hint">PNG, JPEG, or GIF (max 256KB)</span>
      </div>
      <div class="settings-section-title">Display Name</div>
      <FormField>
        <div class="form-field-row">
          <input
            class="form-input"
            type="text"
            aria-label="Display name"
            value={name.value()}
            onInput={(e: InputEvent) => name.edit((e.target as HTMLInputElement).value)}
            onKeyDown={(e: KeyboardEvent) => {
              if (e.key === "Enter") handleSaveName((e.currentTarget as HTMLInputElement).value);
            }}
          />
          <button type="button" class="form-btn-primary" onClick={() => handleSaveName(name.value())}>Save</button>
        </div>
      </FormField>
      <div class="settings-section-title">Status Message</div>
      <FormField>
        <div class="form-field-row">
          <input
            class="form-input"
            type="text"
            aria-label="Status message"
            placeholder="What's on your mind?"
            value={statusMsgInput()}
            onInput={(e: InputEvent) => setStatusMsgInput((e.target as HTMLInputElement).value)}
            onKeyDown={(e: KeyboardEvent) => { if (e.key === "Enter") handleSaveStatusMessage(); }}
          />
          <button type="button" class="form-btn-primary" onClick={handleSaveStatusMessage}>Save</button>
        </div>
      </FormField>
      <div class="settings-section-title">Bio</div>
      <FormField>
        <textarea
          class="form-textarea"
          aria-label="Bio"
          aria-describedby="bio-count"
          rows={3}
          placeholder="Tell your friends about yourself"
          value={bio.value()}
          onInput={(e: InputEvent) => bio.edit((e.target as HTMLTextAreaElement).value)}
        />
        <div class="form-field-row">
          <span id="bio-count" class="bio-count" classList={{ "bio-count-over": bioLength() > MAX_BIO_LEN }}>
            {bioLength()} of {MAX_BIO_LEN} characters
          </span>
          <button type="button" class="form-btn-primary" disabled={bioLength() > MAX_BIO_LEN} onClick={handleSaveBio}>
            Save
          </button>
        </div>
      </FormField>
      <div class="settings-section-title">Game Time</div>
      <Show
        when={(gameTotals() ?? []).length > 0}
        fallback={<div class="game-time-empty">{gameTotals.error ? "Game time could not be read" : "No games played yet"}</div>}
      >
        <ul class="game-time-list" aria-label="Game time">
          <For each={gameTotals()}>
            {(game) => (
              <li class="game-time-row">
                <GameIcon iconKey={game.iconKey} />
                <span class="game-time-name">{game.gameName}</span>
                <span class="game-time-total">{formatDuration(game.totalSecs)}</span>
                <span class="game-time-last">{formatRelativeTime(game.lastPlayed)}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </>
  );
};

export default ProfileTab;
