import { Component, For, Show, createSignal } from "solid-js";
import { commands } from "../../ipc/commands";
import { communityState } from "../../stores/community.store";

interface EventAttendeesProps {
  communityId: string;
  eventId: string;
}

type Attendee = { pseudonymKey: string; status: string };

/**
 * Who answered an event (plan E7.15): opening it reads the Host's
 * `EventAttendees` list, as the Host merged it; names come from the
 * community's member list, the key's start when the member is not listed.
 */
const EventAttendees: Component<EventAttendeesProps> = (props) => {
  const [attendees, setAttendees] = createSignal<Attendee[] | null>(null);
  const [failed, setFailed] = createSignal(false);

  function nameOf(key: string): string {
    const member = communityState.communities[props.communityId]?.members
      .find((m) => m.pseudonymKey === key);
    return member?.displayName ?? `${key.slice(0, 12)}…`;
  }

  function onToggle(open: boolean): void {
    if (!open) return;
    setFailed(false);
    commands.listEventAttendees(props.communityId, props.eventId)
      .then(setAttendees)
      .catch((e) => {
        console.error("Failed to load attendees:", e);
        setFailed(true);
      });
  }

  return (
    <details class="event-card-description" onToggle={(e) => onToggle(e.currentTarget.open)}>
      <summary>Attendees</summary>
      <Show when={!failed()} fallback={<div class="settings-hint">Could not load the attendees.</div>}>
        <Show when={attendees()} fallback={<div class="settings-hint">Loading…</div>}>
          {(list) => (
            <Show when={list().length > 0} fallback={<div class="settings-hint">No answers yet.</div>}>
              <ul>
                <For each={list()}>
                  {(a) => <li>{nameOf(a.pseudonymKey)}: {a.status}</li>}
                </For>
              </ul>
            </Show>
          )}
        </Show>
      </Show>
    </details>
  );
};

export default EventAttendees;
