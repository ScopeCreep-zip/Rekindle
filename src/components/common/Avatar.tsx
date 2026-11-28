import { type Component, Show } from "solid-js";
import type { UserStatus } from "../../stores/auth.store";
import { initials } from "../../utils/initials";

interface AvatarProps {
  displayName: string;
  status?: UserStatus;
  size?: number;
  avatarUrl?: string;
}

const Avatar: Component<AvatarProps> = (props) => {
  const size = () => props.size ?? 32;

  return (
    <Show
      when={props.avatarUrl}
      fallback={
        <div
          class="avatar"
          style={{
            width: `${size()}px`,
            height: `${size()}px`,
            "font-size": `${Math.floor(size() * 0.4)}px`,
          }}
        >
          {initials(props.displayName)}
        </div>
      }
    >
      <img
        class="avatar"
        src={props.avatarUrl}
        alt={props.displayName}
        style={{
          width: `${size()}px`,
          height: `${size()}px`,
          "object-fit": "cover",
          "border-radius": "50%",
        }}
      />
    </Show>
  );
};

export default Avatar;
