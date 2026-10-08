import { createSignal } from "solid-js";
import type { VideoCallVm } from "../video_call/useVideoCallPanel";

/// The single live WebCodecs pipeline for the active community call.
/// Published by `CallPipelineHost` (mounted once at the window root so the
/// call keeps broadcasting while the user navigates between channels) and
/// consumed by the main-pane `CallStage`. Null whenever no community call
/// is active.
const [activePipeline, setActivePipeline] = createSignal<VideoCallVm | null>(null);

export { activePipeline, setActivePipeline };
