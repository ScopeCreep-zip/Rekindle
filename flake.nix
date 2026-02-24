{
  description = "Rekindle — Xfire rebuilt as a decentralized Tauri 2 app";

  inputs = {
    konductor.url = "github:braincraftio/konductor";
    nixpkgs.follows = "konductor/nixpkgs";
    flake-utils.follows = "konductor/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, konductor, ... }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = system: import nixpkgs { inherit system; };
    in
    {
      packages = forAllSystems (system: {
        default = (pkgsFor system).callPackage ./nix/package.nix { };
        rekindle = self.packages.${system}.default;
      });

      overlays.default = final: prev: {
        rekindle = final.callPackage ./nix/package.nix { };
      };

      homeManagerModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.rekindle;
          defaultPkg = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        in
        {
          options.programs.rekindle = {
            enable = lib.mkEnableOption "Rekindle — decentralized gaming social platform";

            package = lib.mkOption {
              type = lib.types.package;
              default = defaultPkg;
              description = "The rekindle package to use.";
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];
          };
        };

      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;

          rekindlePackages = with pkgs; [
            capnproto
            cmake
            libsodium.dev
          ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            alsa-lib.dev
            libopus.dev
          ];

          rekindleLibPath = pkgs.lib.optionalString pkgs.stdenv.isLinux
            (pkgs.lib.makeLibraryPath (with pkgs; [
              libsodium
              libopus
              alsa-lib
            ]));
        in {
          default = pkgs.mkShell {
            name = "rekindle";
            packages = rekindlePackages;
            inputsFrom = [ konductor.devShells.${system}.frontend ];

            env = {
              KONDUCTOR_SHELL = "rekindle";
              SODIUM_USE_PKG_CONFIG = "1";
              REKINDLE_LIB_PATH = rekindleLibPath;
            };
          };
        });
    };
}
