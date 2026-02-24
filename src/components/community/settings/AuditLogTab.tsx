import { Component, For, Show, createSignal, createEffect } from "solid-js";
import { handleGetAuditLog } from "../../../handlers/community.handlers";
import { truncateKey, formatDateTimeSecs, formatAction } from "../../../utils/formatting";
import type { AuditLogEntryDto } from "../../../stores/types";

interface AuditLogTabProps {
  communityId: string;
}

const AuditLogTab: Component<AuditLogTabProps> = (props) => {
  const [entries, setEntries] = createSignal<AuditLogEntryDto[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [hasMore, setHasMore] = createSignal(true);
  const [loaded, setLoaded] = createSignal(false);

  createEffect(() => {
    if (!loaded()) {
      setLoaded(true);
      loadEntries();
    }
  });

  async function loadEntries(): Promise<void> {
    setLoading(true);
    const result = await handleGetAuditLog(props.communityId, undefined, 50);
    setEntries(result);
    setHasMore(result.length >= 50);
    setLoading(false);
  }

  async function loadMore(): Promise<void> {
    const current = entries();
    if (current.length === 0) return;
    const oldest = current[current.length - 1];
    setLoading(true);
    const result = await handleGetAuditLog(props.communityId, oldest.timestamp, 50);
    setEntries((prev) => [...prev, ...result]);
    setHasMore(result.length >= 50);
    setLoading(false);
  }

  return (
    <div class="settings-section">
      <Show when={entries().length === 0 && !loading()}>
        <div class="settings-hint">No audit log entries.</div>
      </Show>
      <div class="audit-log-list">
        <For each={entries()}>
          {(entry) => (
            <div class="audit-log-entry">
              <div class="audit-log-action">{formatAction(entry.action)}</div>
              <div class="audit-log-meta">
                <span class="audit-log-actor">{truncateKey(entry.actorPseudonym)}</span>
                <Show when={entry.target}>
                  <span class="audit-log-target">{truncateKey(entry.target!)}</span>
                </Show>
                <Show when={entry.details}>
                  <span class="audit-log-details">{entry.details}</span>
                </Show>
                <span class="audit-log-time">{formatDateTimeSecs(entry.timestamp)}</span>
              </div>
            </div>
          )}
        </For>
      </div>
      <Show when={hasMore() && entries().length > 0}>
        <button class="form-btn-secondary" onClick={loadMore} disabled={loading()}>
          {loading() ? "Loading..." : "Load More"}
        </button>
      </Show>
    </div>
  );
};

export default AuditLogTab;
