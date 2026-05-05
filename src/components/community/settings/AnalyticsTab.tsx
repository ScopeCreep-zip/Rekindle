import { Component, For, Show, createMemo, createResource } from "solid-js";
import type { Community } from "../../../stores/community.store";
import { commands, type CommunityAnalytics, type DailyTimeseries } from "../../../ipc/commands";

// Architecture §24 — local-only community analytics. Computed entirely
// client-side from SQLite + governance state; never broadcast.

interface AnalyticsTabProps {
  community: Community;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

function maxOf(samples: { value: number }[]): number {
  let max = 0;
  for (const s of samples) max = Math.max(max, s.value);
  return max || 1;
}

function maxOfNumbers(values: number[]): number {
  let max = 0;
  for (const v of values) max = Math.max(max, v);
  return max || 1;
}

const Sparkline: Component<{ series: DailyTimeseries; label: string }> = (props) => {
  const max = createMemo(() => maxOf(props.series.samples));
  // Architecture §32 a11y — collapse the daily series into a single
  // accessible-name string so screen readers read e.g. "Joins per day:
  // 0–14, peak 14 on Apr 28" instead of 30 unlabeled bars.
  const ariaSummary = createMemo(() => {
    const samples = props.series.samples;
    if (samples.length === 0) return props.label;
    const peak = samples.reduce(
      (best, s) => (s.value > best.value ? s : best),
      samples[0],
    );
    const total = samples.reduce((sum, s) => sum + s.value, 0);
    const peakDay = new Date(peak.dayUnixMs).toLocaleDateString();
    return `${props.label}: ${samples.length}-day series, total ${total}, peak ${peak.value} on ${peakDay}`;
  });
  return (
    <div class="analytics-sparkline" role="img" aria-label={ariaSummary()}>
      <div class="analytics-sparkline-label" aria-hidden="true">{props.label}</div>
      <div class="analytics-sparkline-bars" aria-hidden="true">
        <For each={props.series.samples}>
          {(sample) => (
            <div
              class="analytics-sparkline-bar"
              style={{
                height: `${Math.max(2, (sample.value / max()) * 40)}px`,
              }}
              title={`${new Date(sample.dayUnixMs).toDateString()}: ${sample.value}`}
            />
          )}
        </For>
      </div>
    </div>
  );
};

const AnalyticsTab: Component<AnalyticsTabProps> = (props) => {
  const [analytics] = createResource<CommunityAnalytics, string>(
    () => props.community.id,
    (id: string) => commands.getCommunityAnalytics(id),
  );

  return (
    <div class="settings-section">
      <Show when={analytics.loading}>
        <div class="settings-hint">Computing analytics…</div>
      </Show>
      <Show when={analytics.error}>
        <div class="search-panel-error">
          {String(analytics.error)} — analytics requires the
          <code> VIEW_INSIGHTS </code> permission.
        </div>
      </Show>
      <Show when={analytics()}>
        {(data) => (
          <>
            <div class="settings-section-title" id="analytics-members-heading">Members</div>
            <div
              class="analytics-grid"
              role="group"
              aria-labelledby="analytics-members-heading"
            >
              <div class="analytics-card">
                <div class="analytics-card-label">Total members</div>
                <div class="analytics-card-value">{data().members.totalMembers}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Active (7d)</div>
                <div class="analytics-card-value">{data().members.active7d}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Active (30d)</div>
                <div class="analytics-card-value">{data().members.active30d}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Joins (7d)</div>
                <div class="analytics-card-value">{data().members.joins7d}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Leaves (7d)</div>
                <div class="analytics-card-value">{data().members.leaves7d}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">7d retention of 30d</div>
                <div class="analytics-card-value">
                  {(data().members.retention7Of30 * 100).toFixed(0)}%
                </div>
              </div>
            </div>
            <Sparkline series={data().members.activePerDay} label="Active per day (30d)" />
            <Sparkline series={data().members.joinsPerDay} label="Joins per day (30d)" />
            <Sparkline series={data().members.leavesPerDay} label="Leaves per day (30d)" />

            <div class="settings-section-title" id="analytics-channels-heading">Channels</div>
            <div role="group" aria-labelledby="analytics-channels-heading">
            <For each={data().channels}>
              {(channel) => {
                const channelName = createMemo(() =>
                  props.community.channels.find((c) => c.id === channel.channelId)?.name
                    ?? channel.channelId.slice(0, 8),
                );
                return (
                  <div class="analytics-channel-row">
                    <div class="analytics-channel-name">#{channelName()}</div>
                    <div class="analytics-channel-stats">
                      <span>{channel.messages7d} msgs/7d</span>
                      <span>{channel.uniquePosters7d} posters</span>
                      <Show when={channel.peakConcurrentVoice > 0}>
                        <span>peak voice {channel.peakConcurrentVoice}</span>
                      </Show>
                    </div>
                    <Sparkline series={channel.messagesPerDay} label="" />
                  </div>
                );
              }}
            </For>
            </div>

            <div class="settings-section-title" id="analytics-hours-heading">Activity by hour (UTC)</div>
            <div
              class="analytics-hours"
              role="img"
              aria-labelledby="analytics-hours-heading"
            >
              {(() => {
                const max = maxOfNumbers(data().activityByHour.hourCounts);
                return (
                  <For each={data().activityByHour.hourCounts}>
                    {(count, idx) => (
                      <div class="analytics-hour-bar-wrap" title={`Hour ${idx()}: ${count}`}>
                        <div
                          class="analytics-hour-bar"
                          style={{ height: `${Math.max(2, (count / max) * 60)}px` }}
                        />
                        <div class="analytics-hour-label">{idx()}</div>
                      </div>
                    )}
                  </For>
                );
              })()}
            </div>

            <div class="settings-section-title" id="analytics-storage-heading">Storage</div>
            <div
              class="analytics-grid"
              role="group"
              aria-labelledby="analytics-storage-heading"
            >
              <div class="analytics-card">
                <div class="analytics-card-label">Total</div>
                <div class="analytics-card-value">{formatBytes(data().storageUsage.totalBytes)}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Messages</div>
                <div class="analytics-card-value">{formatBytes(data().storageUsage.messageBytes)}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Threads</div>
                <div class="analytics-card-value">{formatBytes(data().storageUsage.threadMessageBytes)}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Pins</div>
                <div class="analytics-card-value">{formatBytes(data().storageUsage.channelPinBytes)}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Voice events</div>
                <div class="analytics-card-value">{formatBytes(data().storageUsage.voiceEventBytes)}</div>
              </div>
              <div class="analytics-card">
                <div class="analytics-card-label">Metadata</div>
                <div class="analytics-card-value">{formatBytes(data().storageUsage.metadataBytes)}</div>
              </div>
            </div>

            <div class="settings-hint">
              Computed in {data().computedInMs} ms · local-only, never broadcast.
            </div>
          </>
        )}
      </Show>
    </div>
  );
};

export default AnalyticsTab;
