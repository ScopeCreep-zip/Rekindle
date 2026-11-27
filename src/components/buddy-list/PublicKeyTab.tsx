import { type Component, createSignal, onMount, Show } from "solid-js";
import {
  handleAddFriendFromCode,
  handleGetFriendCode,
  handleRotateFriendToken,
} from "../../actions/buddy.actions";
import type { FriendCodeView } from "../../ipc/commands/dto";
import OwnRecordsWait from "../common/OwnRecordsWait";
import FriendCodeCard from "./FriendCodeCard";

interface PublicKeyTabProps {
  onClose: () => void;
}

/** The friend-code tab (plan E2.5, D17): our code with its QR and token
 *  rotation, and the field that adds a friend from theirs. */
const PublicKeyTab: Component<PublicKeyTabProps> = (props) => {
  const [ours, setOurs] = createSignal<FriendCodeView | null>(null);
  const [rotating, setRotating] = createSignal(false);
  const [codeError, setCodeError] = createSignal<string | null>(null);
  const [friendCode, setFriendCode] = createSignal("");
  const [message, setMessage] = createSignal("Hey, add me!");
  const [error, setError] = createSignal<string | null>(null);
  const [sending, setSending] = createSignal(false);

  // The Host answers once our profile and mailbox are published, however
  // long the first attach takes (FC1): until then the tab says so. Only a
  // real failure (the records were not published) answers an error, which
  // stays on screen with a retry, never a blank card.
  async function load(): Promise<void> {
    setCodeError(null);
    const result = await handleGetFriendCode();
    if (typeof result === "string") setCodeError(result);
    else setOurs(result);
  }

  onMount(() => void load());

  async function rotate(): Promise<void> {
    setRotating(true);
    setCodeError(null);
    const result = await handleRotateFriendToken();
    setRotating(false);
    if (typeof result === "string") setCodeError(result);
    else if (result !== null) setOurs(result);
  }

  async function handleKeySubmit(e: Event): Promise<void> {
    e.preventDefault();
    const key = friendCode().trim();
    if (!key) return;
    setError(null);
    setSending(true);
    const err = await handleAddFriendFromCode(key, message().trim());
    setSending(false);
    if (err) {
      setError(err);
    } else {
      props.onClose();
    }
  }

  return (
    <div class="add-friend-section">
      <Show
        when={ours()}
        fallback={
          <div class="add-friend-code-status" role="status">
            {codeError() ?? "Publishing your identity…"}
            <Show when={codeError()}>
              <button class="form-btn-primary" type="button" onClick={() => void load()}>
                Try again
              </button>
            </Show>
          </div>
        }
      >
        {(view) => (
          <FriendCodeCard view={view()} rotating={rotating()} onRotate={() => void rotate()} />
        )}
      </Show>
      <Show when={ours() && codeError()}>
        <div class="form-error" role="alert">
          {codeError()}
        </div>
      </Show>

      <div class="add-friend-divider">or paste a friend's code</div>

      <form class="form-group" onSubmit={handleKeySubmit}>
        <input
          class="form-input"
          type="text"
          aria-label="Friend code"
          placeholder="Paste a friend code (rekindle://add/...)"
          value={friendCode()}
          onInput={(e) => setFriendCode(e.currentTarget.value)}
        />
        <input
          class="form-input"
          type="text"
          aria-label="Message (optional)"
          placeholder="Message (optional)"
          value={message()}
          onInput={(e) => setMessage(e.currentTarget.value)}
        />
        <OwnRecordsWait active={sending()} />
        <Show when={error()}>
          <div class="form-error" role="alert">
            {error()}
          </div>
        </Show>
        <button class="form-btn-primary" type="submit" disabled={sending() || !friendCode().trim()}>
          Send Friend Request
        </button>
      </form>
    </div>
  );
};

export default PublicKeyTab;
