import { Component, For, Show, createMemo } from "solid-js";
import type { Channel, Member, VoiceChannelState } from "../../stores/community.store";
import {
  ICON_CHECK,
  ICON_CLOSE,
  ICON_HEADPHONES,
  ICON_MIC,
  ICON_MIC_OFF,
  ICON_PHONE,
} from "../../icons";

interface StagePanelProps {
  channel: Channel;
  voiceChannel?: VoiceChannelState;
  members: Member[];
  myPseudonymKey: string | null;
  isConnectedToChannel: boolean;
  onJoinStage: () => void;
  onLeaveStage: () => void;
  onRequestToSpeak: () => void;
  onApproveRequest: (pseudonymKey: string) => void;
  onDenyRequest: (pseudonymKey: string) => void;
  /** MANAGE_MESSAGES — gates the moderator approve/deny panel. */
  canModerate: boolean;
  /** REQUEST_TO_SPEAK — gates the audience-side hand-raise button.
   *  Without it, the audience member sees the speakers/audience lists
   *  but can't ask to be promoted. */
  canRequestToSpeak: boolean;
}

const StagePanel: Component<StagePanelProps> = (props) => {
  const memberName = (pseudonymKey: string): string =>
    props.members.find((member) => member.pseudonymKey === pseudonymKey)?.displayName
      ?? `${pseudonymKey.slice(0, 12)}...`;

  const speakers = createMemo(() => props.voiceChannel?.speakers ?? props.channel.stageSpeakers ?? []);
  const participants = createMemo(() => props.voiceChannel?.participants ?? []);
  const pendingRequests = createMemo(() => props.voiceChannel?.pendingRequests ?? []);
  const isSpeaker = createMemo(() => {
    const myPseudonymKey = props.myPseudonymKey;
    return Boolean(myPseudonymKey && speakers().includes(myPseudonymKey));
  });
  const hasRaisedHand = createMemo(() => {
    const myPseudonymKey = props.myPseudonymKey;
    return Boolean(myPseudonymKey && pendingRequests().includes(myPseudonymKey));
  });
  const audience = createMemo(() =>
    participants().filter((participant) => !speakers().includes(participant)),
  );

  return (
    <div class="stage-panel">
      <div class="stage-panel-hero">
        <div class="stage-panel-heading">
          <div class="stage-panel-title">{props.channel.name}</div>
          <Show when={props.channel.topic}>
            <div class="stage-panel-topic">{props.channel.topic}</div>
          </Show>
        </div>
        <div class="stage-panel-actions">
          <Show
            when={props.isConnectedToChannel}
            fallback={
              <button class="stage-panel-join-btn" onClick={props.onJoinStage}>
                <span class="nf-icon">{ICON_PHONE}</span>
                Join stage
              </button>
            }
          >
            <button class="stage-panel-leave-btn" onClick={props.onLeaveStage}>
              <span class="nf-icon">{ICON_HEADPHONES}</span>
              Leave stage
            </button>
          </Show>
          <Show when={props.isConnectedToChannel && !isSpeaker() && props.canRequestToSpeak}>
            <button
              class="stage-panel-request-btn"
              onClick={props.onRequestToSpeak}
              disabled={hasRaisedHand()}
              aria-label={hasRaisedHand() ? "Hand raised — waiting for moderator" : "Raise hand to request speaking"}
              aria-pressed={hasRaisedHand()}
            >
              <span class="nf-icon" aria-hidden="true">{ICON_MIC_OFF}</span>
              {hasRaisedHand() ? "Hand raised — waiting for moderator" : "Request to speak"}
            </button>
          </Show>
        </div>
      </div>

      <div class="stage-panel-status">
        <Show when={isSpeaker()} fallback={<span>You are in the audience.</span>}>
          <span>You are on stage.</span>
        </Show>
        <Show when={props.channel.stageModerator}>
          <span>Moderated by {memberName(props.channel.stageModerator!)}</span>
        </Show>
      </div>

      <div class="stage-panel-grid">
        <div class="stage-panel-section">
          <div class="stage-panel-section-title">
            <span class="nf-icon">{ICON_MIC}</span>
            Speakers ({speakers().length})
          </div>
          <Show when={speakers().length > 0} fallback={<div class="stage-panel-empty">No speakers yet.</div>}>
            <div class="stage-panel-list">
              <For each={speakers()}>
                {(speaker) => (
                  <div class="stage-panel-member-card">
                    <div class="stage-panel-member-name">{memberName(speaker)}</div>
                    <div class="stage-panel-member-meta">{speaker.slice(0, 12)}...</div>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>

        <div class="stage-panel-section">
          <div class="stage-panel-section-title">
            <span class="nf-icon">{ICON_HEADPHONES}</span>
            Audience ({audience().length})
          </div>
          <Show when={audience().length > 0} fallback={<div class="stage-panel-empty">No audience members connected.</div>}>
            <div class="stage-panel-list">
              <For each={audience()}>
                {(participant) => (
                  <div class="stage-panel-member-card">
                    <div class="stage-panel-member-name">{memberName(participant)}</div>
                    <div class="stage-panel-member-meta">{participant.slice(0, 12)}...</div>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>
      </div>

      <Show when={props.canModerate}>
        <div class="stage-panel-section stage-panel-requests">
          <div class="stage-panel-section-title">
            <span class="nf-icon">{ICON_MIC_OFF}</span>
            Raised hands ({pendingRequests().length})
          </div>
          <Show when={pendingRequests().length > 0} fallback={<div class="stage-panel-empty">No pending speak requests.</div>}>
            <div class="stage-panel-list">
              <For each={pendingRequests()}>
                {(requester) => (
                  <div class="stage-panel-request-card">
                    <div>
                      <div class="stage-panel-member-name">{memberName(requester)}</div>
                      <div class="stage-panel-member-meta">{requester.slice(0, 12)}...</div>
                    </div>
                    <div class="stage-panel-request-actions">
                      <button
                        class="stage-panel-approve-btn"
                        onClick={() => props.onApproveRequest(requester)}
                        title="Approve request"
                        aria-label={`Approve speak request from ${memberName(requester)}`}
                      >
                        <span class="nf-icon" aria-hidden="true">{ICON_CHECK}</span>
                      </button>
                      <button
                        class="stage-panel-deny-btn"
                        onClick={() => props.onDenyRequest(requester)}
                        title="Deny request"
                        aria-label={`Deny speak request from ${memberName(requester)}`}
                      >
                        <span class="nf-icon" aria-hidden="true">{ICON_CLOSE}</span>
                      </button>
                    </div>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default StagePanel;
