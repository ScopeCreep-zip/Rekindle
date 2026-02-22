import { Component } from "solid-js";
import { colorIntToHex } from "../../utils/color";

interface RoleTagProps {
  name: string;
  color?: number;
}

const RoleTag: Component<RoleTagProps> = (props) => {
  const hex = () => {
    const c = props.color ?? 0;
    return c ? colorIntToHex(c) : undefined;
  };

  return (
    <span
      class={hex() ? "role-tag" : "role-tag role-tag-default"}
      style={hex() ? {
        background: `color-mix(in srgb, ${hex()} 25%, transparent)`,
        color: hex(),
      } : {}}
    >
      {props.name}
    </span>
  );
};

export default RoleTag;
