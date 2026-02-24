import { Component, createSignal, createEffect } from "solid-js";
import Modal from "../common/Modal";
import { handleCreateEvent, handleEditEvent } from "../../handlers/community.handlers";

interface CreateEventModalProps {
  isOpen: boolean;
  communityId: string;
  onClose: () => void;
  isEditing?: boolean;
  eventId?: string;
  initialTitle?: string;
  initialDescription?: string;
  initialStartTime?: number;
  initialEndTime?: number;
  initialMaxAttendees?: number;
}

function timestampToDate(ts: number): string {
  const d = new Date(ts * 1000);
  return d.toISOString().split("T")[0];
}

function timestampToTime(ts: number): string {
  const d = new Date(ts * 1000);
  return d.toTimeString().slice(0, 5);
}

const CreateEventModal: Component<CreateEventModalProps> = (props) => {
  const [title, setTitle] = createSignal("");
  const [description, setDescription] = createSignal("");
  const [startDate, setStartDate] = createSignal("");
  const [startTime, setStartTime] = createSignal("");
  const [endDate, setEndDate] = createSignal("");
  const [endTime, setEndTime] = createSignal("");
  const [maxAttendees, setMaxAttendees] = createSignal("");
  const [validationError, setValidationError] = createSignal("");

  createEffect(() => {
    if (props.isOpen) {
      if (props.isEditing) {
        setTitle(props.initialTitle ?? "");
        setDescription(props.initialDescription ?? "");
        if (props.initialStartTime) {
          setStartDate(timestampToDate(props.initialStartTime));
          setStartTime(timestampToTime(props.initialStartTime));
        } else {
          setStartDate("");
          setStartTime("");
        }
        if (props.initialEndTime) {
          setEndDate(timestampToDate(props.initialEndTime));
          setEndTime(timestampToTime(props.initialEndTime));
        } else {
          setEndDate("");
          setEndTime("");
        }
        setMaxAttendees(props.initialMaxAttendees != null ? String(props.initialMaxAttendees) : "");
      } else {
        setTitle("");
        setDescription("");
        setStartDate("");
        setStartTime("");
        setEndDate("");
        setEndTime("");
        setMaxAttendees("");
      }
      setValidationError("");
    }
  });

  function parseTimestamp(date: string, time: string): number | undefined {
    if (!date || !time) return undefined;
    const d = new Date(`${date}T${time}`);
    if (isNaN(d.getTime())) return undefined;
    return Math.floor(d.getTime() / 1000);
  }

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const t = title().trim();
    if (!t) return;

    const start = parseTimestamp(startDate(), startTime());
    if (!start) return;

    const end = parseTimestamp(endDate(), endTime());

    // Validate end > start if both provided
    if (end && end <= start) {
      setValidationError("End time must be after start time");
      return;
    }
    setValidationError("");

    const max = maxAttendees().trim() ? parseInt(maxAttendees().trim(), 10) : undefined;

    if (props.isEditing && props.eventId) {
      await handleEditEvent(
        props.communityId,
        props.eventId,
        t,
        description().trim(),
        start,
        end,
        undefined,
        max && !isNaN(max) ? max : undefined,
      );
    } else {
      await handleCreateEvent(
        props.communityId,
        t,
        description().trim(),
        start,
        end,
        undefined,
        max && !isNaN(max) ? max : undefined,
      );
    }
    props.onClose();
  }

  const isValid = () => title().trim() && startDate() && startTime();

  return (
    <Modal isOpen={props.isOpen} title={props.isEditing ? "Edit Event" : "Create Event"} onClose={props.onClose}>
      <form class="form-group" onSubmit={handleSubmit}>
        <input
          class="form-input"
          type="text"
          placeholder="Event title..."
          value={title()}
          onInput={(e) => setTitle(e.currentTarget.value)}
        />
        <textarea
          class="form-input"
          placeholder="Description (optional)"
          value={description()}
          onInput={(e) => setDescription(e.currentTarget.value)}
          rows={3}
        />
        <label class="form-label">Start</label>
        <div class="form-row">
          <input
            class="form-input"
            type="date"
            value={startDate()}
            onInput={(e) => setStartDate(e.currentTarget.value)}
          />
          <input
            class="form-input"
            type="time"
            value={startTime()}
            onInput={(e) => setStartTime(e.currentTarget.value)}
          />
        </div>
        <label class="form-label">End (optional)</label>
        <div class="form-row">
          <input
            class="form-input"
            type="date"
            value={endDate()}
            onInput={(e) => setEndDate(e.currentTarget.value)}
          />
          <input
            class="form-input"
            type="time"
            value={endTime()}
            onInput={(e) => setEndTime(e.currentTarget.value)}
          />
        </div>
        <input
          class="form-input"
          type="number"
          placeholder="Max attendees (optional)"
          value={maxAttendees()}
          onInput={(e) => setMaxAttendees(e.currentTarget.value)}
          min={1}
        />
        {validationError() && <div class="form-error">{validationError()}</div>}
        <button class="form-btn-primary" type="submit" disabled={!isValid()}>
          {props.isEditing ? "Save Changes" : "Create Event"}
        </button>
      </form>
    </Modal>
  );
};

export default CreateEventModal;
