import { Component, For, Show, createMemo, createSignal } from "solid-js";
import { groupDmState } from "../../stores/group-dm.store";
import {
  handleAcceptGroupDm,
  handleDeclineGroupDm,
  handleOpenGroupChat,
} from "../../actions/group-dm.actions";

// The buddy list's group DMs (plan E2.6): invites we have not answered,
// with Accept (reviewed by the Host, TrustChanging) and Decline, then the
// groups we are in, each opening the chat window's group mode.
const GroupDmList: Component = () => {
  const [error, setError] = createSignal<string | null>(null);
  const all = createMemo(() => Object.values(groupDmState.groups));
  const invites = createMemo(() => all().filter((g) => g.pending));
  const groups = createMemo(() =>
    all()
      .filter((g) => !g.pending)
      .sort((a, b) => a.name.localeCompare(b.name)),
  );

  async function run(answer: Promise<string | null>): Promise<void> {
    setError(null);
    const err = await answer;
    if (err) setError(err);
  }

  return (
    <Show when={all().length > 0}>
      <section class="group-dm-section" aria-label="Group chats">
        <div class="pending-header">Group Chats</div>
        <Show when={error()}>
          <div class="form-error" role="alert">{error()}</div>
        </Show>
        <For each={invites()}>
          {(invite) => (
            <div class="pending-item" role="group" aria-label={`Invite to ${invite.name}`}>
              <div class="pending-info">
                <span class="buddy-name">{invite.name}</span>
                <span class="pending-message">
                  {invite.members.map((m) => m.label).join(", ")}
                </span>
              </div>
              <button
                class="pending-btn-accept"
                aria-label={`Accept invite to ${invite.name}`}
                onClick={() => void run(handleAcceptGroupDm(invite.groupId))}
              >
                Accept
              </button>
              <button
                class="pending-btn-reject"
                aria-label={`Decline invite to ${invite.name}`}
                onClick={() => void run(handleDeclineGroupDm(invite.groupId))}
              >
                Decline
              </button>
            </div>
          )}
        </For>
        <For each={groups()}>
          {(group) => (
            <button
              class="group-dm-row"
              aria-label={`Open group chat ${group.name}, ${group.members.length} members`}
              onDblClick={() => handleOpenGroupChat(group.groupId)}
              onKeyDown={(e) => e.key === "Enter" && handleOpenGroupChat(group.groupId)}
            >
              <span class="buddy-name">{group.name}</span>
              <span class="group-dm-count">{group.members.length}</span>
            </button>
          )}
        </For>
      </section>
    </Show>
  );
};

export default GroupDmList;
