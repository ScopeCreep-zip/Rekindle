import { type Component, createMemo, onCleanup, Show } from "solid-js";
import { voiceState } from "../../../stores/voice.store";
import Avatar from "../../common/Avatar";
import SkinGlyph from "../../common/SkinGlyph";
import type { CallTile } from "./useCallStage";

/// One gallery cell: a live video surface (remote decoder canvas or local
/// preview) when the participant is sending video, else an avatar. Speaking
/// ring + muted badge are read reactively so the tile updates without the
/// gallery membership recomputing.
const ParticipantTile: Component<{
  tile: CallTile;
  spotlighted?: boolean;
  onSpotlight?: () => void;
}> = (props) => {
  const participant = createMemo(() => {
    const key = props.tile.publicKey;
    if (!key) return undefined;
    return voiceState.participants.find((p) => p.publicKey === key);
  });

  const isSpeaking = createMemo(() => participant()?.isSpeaking ?? false);
  const isMuted = createMemo(() =>
    props.tile.isLocal ? voiceState.isMuted : (participant()?.isMuted ?? false)
  );
  const name = createMemo(() => participant()?.displayName ?? props.tile.displayName);

  return (
    <div
      class="call-stage-tile"
      classList={{
        "call-stage-tile-spotlight": props.spotlighted,
        "call-stage-tile-speaking": isSpeaking(),
        // Mirror only the SELF CAMERA surface — screens must read
        // normally (mirrored text), remotes are never mirrored.
        "call-stage-tile-mirror": props.tile.mirror === true,
        "call-stage-tile-screen": props.tile.isScreen,
      }}
    >
      <Show
        when={props.tile.canvas}
        fallback={
          <Show
            when={props.tile.bindVideo}
            fallback={
              <div class="call-stage-tile-avatar">
                <Avatar
                  displayName={name()}
                  avatarUrl={props.tile.avatarUrl}
                  size={props.spotlighted ? 128 : 72}
                />
              </div>
            }
          >
            <video
              class="call-stage-tile-video"
              autoplay
              muted
              playsinline
              ref={(el) => {
                props.tile.bindVideo?.(el);
                onCleanup(() => props.tile.bindVideo?.(null));
              }}
            />
          </Show>
        }
      >
        <div
          class="call-stage-tile-canvas"
          ref={(el) => {
            const canvas = props.tile.canvas;
            if (canvas) el.appendChild(canvas);
          }}
        />
      </Show>

      <button
        type="button"
        class="call-stage-tile-spotlight-btn"
        aria-pressed={!!props.spotlighted}
        aria-label={`Spotlight ${name()}`}
        onDblClick={() => props.onSpotlight?.()}
        onClick={(e) => {
          if (e.detail === 0) props.onSpotlight?.();
        }}
      />
      <div class="call-stage-tile-overlay">
        <Show when={props.tile.isScreen}>
          <SkinGlyph name="screen-share" class="call-stage-tile-badge" />
        </Show>
        <Show when={isMuted()}>
          <SkinGlyph name="mic.off" class="call-stage-tile-badge call-stage-tile-badge-muted" />
        </Show>
        <span class="call-stage-tile-name">{name()}</span>
      </div>
    </div>
  );
};

export default ParticipantTile;
