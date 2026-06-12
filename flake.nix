{
  description = "Rekindle — Xfire rebuilt as a decentralized Tauri 2 app";

  inputs = {
    konductor.url = "github:braincraftio/konductor";
    nixpkgs.follows = "konductor/nixpkgs";
    flake-utils.follows = "konductor/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, konductor, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        rekindlePackages = with pkgs; [
          capnproto
          cmake
          libsodium.dev
        ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
          alsa-lib.dev
          libopus.dev
          dbus.dev
          # Native video capture (crates/rekindle-video-capture):
          # gstreamer-rs needs the dev headers; the plugins provide
          # v4l2src/jpegdec/vp8enc at dev-shell runtime.
          gst_all_1.gstreamer.dev
          gst_all_1.gst-plugins-base.dev
          gst_all_1.gst-plugins-good
        ];

        # Runtime library path for Nix-provided shared libs on Linux.
        rekindleLibPath = pkgs.lib.optionalString pkgs.stdenv.isLinux
          (pkgs.lib.makeLibraryPath (with pkgs; [
            libsodium
            libopus
            alsa-lib
            dbus
            gst_all_1.gstreamer
            gst_all_1.gst-plugins-base
            gst_all_1.gst-plugins-good
          ]));

        # GStreamer plugin search path for the dev shell — the system
        # plugin dirs are invisible from Nix-provided libgstreamer.
        rekindleGstPluginPath = pkgs.lib.optionalString pkgs.stdenv.isLinux
          (pkgs.lib.makeSearchPath "lib/gstreamer-1.0" (with pkgs; [
            gst_all_1.gstreamer
            gst_all_1.gst-plugins-base
            gst_all_1.gst-plugins-good
          ]));

      in {
        devShells.default = pkgs.mkShell {
          name = "rekindle";
          packages = rekindlePackages;
          inputsFrom = [ konductor.devShells.${system}.frontend ];

          # Use env instead of shellHook — direnv's use flake does NOT
          # execute shellHook, only captures env attrs.
          env = {
            KONDUCTOR_SHELL = "rekindle";
            SODIUM_USE_PKG_CONFIG = "1";
            REKINDLE_LIB_PATH = rekindleLibPath;
            LD_LIBRARY_PATH = rekindleLibPath;
            GST_PLUGIN_SYSTEM_PATH_1_0 = rekindleGstPluginPath;
          };
        };
      }
    );
}
