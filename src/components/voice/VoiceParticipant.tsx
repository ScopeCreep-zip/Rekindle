import { Component } from "solid-js";
import { VoiceParticipant as VoiceParticipantType } from "../../stores/voice.store";

interface VoiceParticipantProps {
  participant: VoiceParticipantType;
}

const VoiceParticipantItem: Component<VoiceParticipantProps> = (props) => {
  return (
    <div
      class={`voice-participant ${props.participant.isSpeaking ? "voice-participant-speaking" : ""}`}
    >
      <div class={props.participant.isSpeaking ? "voice-speaking-ring" : "voice-silent-ring"} />
      <span class="voice-participant-name">
        {props.participant.displayName}
        {props.participant.isMuted && " (muted)"}
      </span>
    </div>
  );
};

export default VoiceParticipantItem;
