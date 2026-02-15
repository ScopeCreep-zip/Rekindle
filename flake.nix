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
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };

        rekindlePackages = with pkgs; [
          capnproto
          cmake
          libsodium.dev
        ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
          alsa-lib.dev
          libopus.dev
        ];

        # Runtime library path for Nix-provided shared libs (non-.dev outputs).
        rekindleLibPath = pkgs.lib.makeLibraryPath (with pkgs; [
          libsodium
          libopus
          alsa-lib
        ]);

        shellConfig = {
          packages = rekindlePackages;
        };

      in {
        devShells = {
          default = pkgs.mkShell (shellConfig // {
            name = "rekindle";
            inputsFrom = [ konductor.devShells.${system}.frontend ];
            # Use `env` instead of `shellHook` — direnv's `use flake` captures
            # env attrs via `nix print-dev-env` but does NOT execute shellHook.
            env = {
              KONDUCTOR_SHELL = "rekindle";
              # Force libsodium-sys-stable to use pkg-config instead of building
              # from source (Nix's gcc wrapper strips SIMD/AVX flags).
              SODIUM_USE_PKG_CONFIG = "1";
              # Expose the lib path so .envrc can prepend it to LD_LIBRARY_PATH.
              REKINDLE_LIB_PATH = rekindleLibPath;
            };
          });

          frontend = self.devShells.${system}.default;
        };
      }
    );
}
