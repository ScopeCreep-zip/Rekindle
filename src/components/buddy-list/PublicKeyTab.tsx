import { Component, Show, createSignal, onMount } from "solid-js";
import type { FriendCodeView } from "../../ipc/commands/dto";
import {
  handleAddFriendFromCode,
  handleGetFriendCode,
  handleRotateFriendToken,
} from "../../actions/buddy.actions";
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

  onMount(async () => {
    const result = await handleGetFriendCode();
    if (typeof result === "string") setCodeError(result);
    else setOurs(result);
  });

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
    const err = await handleAddFriendFromCode(key, message().trim());
    if (err) {
      setError(err);
    } else {
      props.onClose();
    }
  }

  return (
    <div class="add-friend-section">
      <Show when={ours()} fallback={<div class="add-friend-code-status" role="status">{codeError() ?? "Reading your friend code…"}</div>}>
        {(view) => <FriendCodeCard view={view()} rotating={rotating()} onRotate={() => void rotate()} />}
      </Show>
      <Show when={ours() && codeError()}>
        <div class="form-error" role="alert">{codeError()}</div>
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
        <Show when={error()}>
          <div class="form-error" role="alert">{error()}</div>
        </Show>
        <button class="form-btn-primary" type="submit" disabled={!friendCode().trim()}>
          Send Friend Request
        </button>
      </form>
    </div>
  );
};

export default PublicKeyTab;
