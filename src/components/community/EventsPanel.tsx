import { Component, For, Show, createSignal, createMemo, onMount } from "solid-js";
import { communityState } from "../../stores/community.store";
import type { CommunityEvent, EventRsvp } from "../../stores/community.store";
import { handleRsvpEvent, handleDeleteEvent, handleCancelEvent, handleLoadEvents } from "../../handlers/community.handlers";

interface EventsPanelProps {
  communityId: string;
  myPseudonymKey: string | null;
  onCreateEvent: () => void;
  onEditEvent?: (event: CommunityEvent) => void;
}

type StatusFilter = "upcoming" | "active" | "past" | "all";

const userTimezone = Intl.DateTimeFormat().resolvedOptions().timeZone;

function formatEventTime(timestamp: number): string {
  const d = new Date(timestamp * 1000);
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function formatTimeUntil(timestamp: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = timestamp - now;
  if (diff <= 0) return "Started";
  if (diff < 3600) return `In ${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `In ${Math.floor(diff / 3600)}h`;
  return `In ${Math.floor(diff / 86400)}d`;
}

function myRsvpStatus(rsvps: EventRsvp[], myKey: string | null): string | null {
  if (!myKey) return null;
  const rsvp = rsvps.find((r) => r.pseudonymKey === myKey);
  return rsvp?.status ?? null;
}

function rsvpCount(rsvps: EventRsvp[], status: string): number {
  return rsvps.filter((r) => r.status === status).length;
}

function statusBadgeClass(status: string): string {
  switch (status) {
    case "active": return "event-status-badge event-status-active";
    case "completed": return "event-status-badge event-status-completed";
    case "canceled": return "event-status-badge event-status-canceled";
    default: return "event-status-badge event-status-scheduled";
  }
}

const EventCard: Component<{
  event: CommunityEvent;
  communityId: string;
  myPseudonymKey: string | null;
  onEditEvent?: (event: CommunityEvent) => void;
}> = (props) => {
  const myStatus = createMemo(() => myRsvpStatus(props.event.rsvps, props.myPseudonymKey));
  const goingCount = createMemo(() => rsvpCount(props.event.rsvps, "going"));
  const isInteractable = createMemo(() =>
    props.event.status === "scheduled" || props.event.status === "active"
  );
  const isCreator = createMemo(() => props.event.creatorPseudonym === props.myPseudonymKey);

  function handleRsvp(status: string): void {
    handleRsvpEvent(props.communityId, props.event.id, status);
  }

  function handleDelete(): void {
    handleDeleteEvent(props.communityId, props.event.id);
  }

  function handleCancel(): void {
    handleCancelEvent(props.communityId, props.event.id);
  }

  return (
    <div class={`event-card ${props.event.status === "canceled" ? "event-card-canceled" : ""}`}>
      <div class="event-card-header">
        <div class="event-card-title">{props.event.title}</div>
        <span class={statusBadgeClass(props.event.status)}>{props.event.status}</span>
      </div>
      <Show when={props.event.description}>
        <div class="event-card-description">{props.event.description}</div>
      </Show>
      <div class="event-card-meta">
        <span>{formatEventTime(props.event.startTime)}</span>
        <Show when={props.event.endTime}>
          <span>- {formatEventTime(props.event.endTime!)}</span>
        </Show>
        <span class="event-timezone">{userTimezone}</span>
        <Show when={props.event.status === "scheduled"}>
          <span class="event-countdown">{formatTimeUntil(props.event.startTime)}</span>
        </Show>
      </div>
      <Show when={isInteractable()}>
        <div class="event-rsvp-row">
          <button
            class={`event-rsvp-btn ${myStatus() === "going" ? "event-rsvp-active" : ""}`}
            onClick={() => handleRsvp("going")}
          >
            Going
          </button>
          <button
            class={`event-rsvp-btn ${myStatus() === "maybe" ? "event-rsvp-active" : ""}`}
            onClick={() => handleRsvp("maybe")}
          >
            Maybe
          </button>
          <button
            class={`event-rsvp-btn ${myStatus() === "declined" ? "event-rsvp-active" : ""}`}
            onClick={() => handleRsvp("declined")}
          >
            Decline
          </button>
          <span class="event-rsvp-count">
            {goingCount()} going
            <Show when={props.event.maxAttendees}>
              {` / ${props.event.maxAttendees}`}
            </Show>
          </span>
        </div>
      </Show>
      <Show when={!isInteractable()}>
        <div class="event-rsvp-row">
          <span class="event-rsvp-count">
            {goingCount()} attended
          </span>
        </div>
      </Show>
      <Show when={isCreator()}>
        <div class="event-card-actions">
          <Show when={isInteractable() && props.onEditEvent}>
            <button class="form-btn-secondary" onClick={() => props.onEditEvent!(props.event)}>
              Edit
            </button>
          </Show>
          <Show when={isInteractable()}>
            <button class="form-btn-secondary" onClick={handleCancel}>
              Cancel Event
            </button>
          </Show>
          <button class="form-btn-danger" onClick={handleDelete}>
            Delete Event
          </button>
        </div>
      </Show>
    </div>
  );
};

const EventsPanel: Component<EventsPanelProps> = (props) => {
  const [loaded, setLoaded] = createSignal(false);
  const [filter, setFilter] = createSignal<StatusFilter>("upcoming");

  onMount(async () => {
    await handleLoadEvents(props.communityId);
    setLoaded(true);
  });

  const events = createMemo(() => {
    const community = communityState.communities[props.communityId];
    if (!community) return [];
    const all = [...(community.events ?? [])].sort((a, b) => a.startTime - b.startTime);
    const f = filter();
    if (f === "all") return all;
    if (f === "upcoming") return all.filter((e) => e.status === "scheduled");
    if (f === "active") return all.filter((e) => e.status === "active");
    // "past" shows completed + canceled
    return all.filter((e) => e.status === "completed" || e.status === "canceled");
  });

  return (
    <div class="event-list">
      <div class="form-row">
        <span class="form-label">Community Events</span>
        <button class="form-btn-primary" onClick={props.onCreateEvent}>
          + New Event
        </button>
      </div>
      <div class="event-filter-row">
        <button
          class={`event-filter-btn ${filter() === "upcoming" ? "event-filter-active" : ""}`}
          onClick={() => setFilter("upcoming")}
        >
          Upcoming
        </button>
        <button
          class={`event-filter-btn ${filter() === "active" ? "event-filter-active" : ""}`}
          onClick={() => setFilter("active")}
        >
          Active
        </button>
        <button
          class={`event-filter-btn ${filter() === "past" ? "event-filter-active" : ""}`}
          onClick={() => setFilter("past")}
        >
          Past
        </button>
        <button
          class={`event-filter-btn ${filter() === "all" ? "event-filter-active" : ""}`}
          onClick={() => setFilter("all")}
        >
          All
        </button>
      </div>
      <Show when={loaded()} fallback={<div class="event-empty">Loading events...</div>}>
        <Show when={events().length > 0} fallback={<div class="event-empty">No events</div>}>
          <For each={events()}>
            {(event) => (
              <EventCard
                event={event}
                communityId={props.communityId}
                myPseudonymKey={props.myPseudonymKey}
                onEditEvent={props.onEditEvent}
              />
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
};

export default EventsPanel;
