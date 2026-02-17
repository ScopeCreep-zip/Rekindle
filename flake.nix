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
        ];

        # Runtime library path for Nix-provided shared libs on Linux.
        rekindleLibPath = pkgs.lib.optionalString pkgs.stdenv.isLinux
          (pkgs.lib.makeLibraryPath (with pkgs; [
            libsodium
            libopus
            alsa-lib
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
          };
        };
      }
    );
}
