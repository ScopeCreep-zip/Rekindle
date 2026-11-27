import type { Component } from "solid-js";
import type { RoleTagColours } from "../../stores/community.store";
import { colorIntToHex } from "../../utils/color";

interface RoleTagProps {
  name: string;
  color?: number;
  /** The name's text colour per skin base, worked out by the Host from
   *  `color` (plan E7.3, SC 1.4.3); the stylesheet picks the active base's. */
  tagColours?: RoleTagColours;
}

const RoleTag: Component<RoleTagProps> = (props) => {
  const hex = () => {
    const c = props.color ?? 0;
    return c && props.tagColours ? colorIntToHex(c) : undefined;
  };

  return (
    <span
      class={hex() ? "role-tag role-tag-coloured" : "role-tag role-tag-default"}
      style={hex() && props.tagColours ? {
        background: `color-mix(in srgb, ${hex()} 25%, transparent)`,
        "--role-tag-dark": colorIntToHex(props.tagColours.dark, "#000000"),
        "--role-tag-light": colorIntToHex(props.tagColours.light, "#000000"),
      } : {}}
    >
      {props.name}
    </span>
  );
};

export default RoleTag;
