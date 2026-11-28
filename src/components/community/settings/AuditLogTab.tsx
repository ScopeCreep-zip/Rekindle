import { type Component, createEffect, createSignal, For, Show } from "solid-js";
import { handleGetAuditLog } from "../../../actions/community.actions";
import type { CommunityAuditEntry } from "../../../ipc/commands/dto";
import { formatAction, formatDateTimeSecs } from "../../../utils/formatting";

interface AuditLogTabProps {
  communityId: string;
}

const AuditLogTab: Component<AuditLogTabProps> = (props) => {
  const [entries, setEntries] = createSignal<CommunityAuditEntry[]>([]);
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
                <span class="audit-log-actor">{entry.actorName}</span>
                <Show when={entry.targetName}>
                  {(name) => <span class="audit-log-target">{name()}</span>}
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
        <button type="button" class="form-btn-secondary" onClick={loadMore} disabled={loading()}>
          {loading() ? "Loading..." : "Load More"}
        </button>
      </Show>
    </div>
  );
};

export default AuditLogTab;
