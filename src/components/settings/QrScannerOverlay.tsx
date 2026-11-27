import { type Component, createSignal, onCleanup, Show } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { returnFocusToOpener } from "../../utils/dialog_focus";
import QrScanner from "qr-scanner";
import { commands } from "../../ipc/commands";

interface QrScannerOverlayProps {
  onResult: (decoded: string) => void;
  onClose: () => void;
}

/**
 * Architecture §28.4 / Phase 7 W24 line 4122 — camera-based QR scanner
 * for the new-device side of the cross-device pairing flow.
 *
 * Uses the `qr-scanner` library (Nimiq, ~16kB gzipped). Renders a
 * fullscreen overlay with a `<video>` element, requests camera
 * permission via `getUserMedia`, decodes via the WebView's native
 * `BarcodeDetector` when available (Chromium/WebView2; WebKitGTK
 * 2.46+) and falls back to the bundled WASM decoder otherwise.
 *
 * On a successful decode the parent receives the decoded string via
 * `onResult` and is responsible for closing the overlay.
 *
 * Camera permission: macOS requires `NSCameraUsageDescription` in the
 * Info.plist; Linux WebKitGTK requires `enable_media_stream` on the
 * webview attributes (set via Tauri 2.x `webview_attributes`).
 */
const QrScannerOverlay: Component<QrScannerOverlayProps> = (props) => {
  const [error, setError] = createSignal<string | null>(null);
  let scanner: QrScanner | null = null;

  // The video element mounts inside the dialog's portal; the scanner
  // starts once it exists.
  function startScanner(video: HTMLVideoElement): void {
    if (scanner) return;
    scanner = new QrScanner(
      video,
      (result) => {
        // qr-scanner returns a `ScanResult` with `.data` in 1.4+.
        const decoded = result.data;
        if (decoded) {
          props.onResult(decoded);
        }
      },
      {
        // Highlight a detection box so users know they're aligned.
        highlightScanRegion: true,
        highlightCodeOutline: true,
        // Prefer the rear camera on hardware that has multiple
        // (laptops typically have only one — qr-scanner falls back
        // gracefully).
        preferredCamera: "environment",
      }
    );
    // A backend-native camera session owns the device exclusively —
    // and a busy camera surfaces NO getUserMedia error: the scanner
    // would sit on silent black, never decoding, with nothing to
    // diagnose. Pre-empt instead.
    commands
      .nativeVideoActive()
      .catch(() => null)
      .then((active) => {
        if (active) {
          setError("Camera is in use by the current call");
          return;
        }
        scanner?.start().catch((e: unknown) => {
          const msg = e instanceof Error ? e.message : String(e);
          console.error("QR scanner start failed:", msg);
          setError(`Could not access camera: ${msg}`);
        });
      });
  }

  onCleanup(() => {
    if (scanner) {
      scanner.stop();
      scanner.destroy();
      scanner = null;
    }
  });

  return (
    // Wiring guide §13c: a modal dialog (over Add Device) traps focus, Esc
    // closes it, and focus goes back to the button that opened it.
    <Dialog open={true} onOpenChange={(open) => !open && props.onClose()} modal={true}>
      <Dialog.Portal>
        <div class="qr-scanner-overlay">
          <Dialog.Content class="qr-scanner-overlay-card" {...returnFocusToOpener()}>
            <div class="qr-scanner-overlay-header">
              <Dialog.Title class="form-field-label">Scan pairing QR</Dialog.Title>
              <Dialog.CloseButton class="form-btn-secondary">Close</Dialog.CloseButton>
            </div>
            <Show when={!error()} fallback={<div class="search-panel-error">{error()}</div>}>
              <video ref={startScanner} class="qr-scanner-video" autoplay playsinline muted />
              <div class="settings-hint">
                Hold the existing device's QR code in front of the camera. Decoding happens locally
                — nothing is sent over the network.
              </div>
            </Show>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
};

export default QrScannerOverlay;
