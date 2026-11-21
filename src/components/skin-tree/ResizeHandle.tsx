import { Component } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Edge } from "../../ipc/commands/dto_skin_tree";

/// Each pack edge's OS resize direction (Tauri `ResizeDirection`).
const DIRECTION = {
  top: "North",
  bottom: "South",
  left: "West",
  right: "East",
  topLeft: "NorthWest",
  topRight: "NorthEast",
  bottomLeft: "SouthWest",
  bottomRight: "SouthEast",
} as const;

/**
 * N14 (design §3): a pack's resize strip or gripper. A press starts the
 * OS's resize drag from that edge; the hit area is the Host's rule
 * (`::before`, yielding to the interactive nodes it overlaps). Keyboard
 * resizing is the OS's, so it is hidden from assistive technology.
 */
const ResizeHandle: Component<{ id: string; edge: Edge }> = (props) => (
  <div
    data-skin-node={props.id}
    class="skin-resize"
    aria-hidden="true"
    onPointerDown={(e) => {
      if (e.button === 0) void getCurrentWindow().startResizeDragging(DIRECTION[props.edge]);
    }}
  />
);

export default ResizeHandle;
