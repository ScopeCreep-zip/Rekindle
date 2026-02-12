{
  description = "Rekindle — Xfire rebuilt as a decentralized Tauri 2 app";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    konductor.url = "github:braincraftio/konductor";
  };

  outputs = { self, nixpkgs, konductor, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems f;
    in
    {
      devShells = forAllSystems (system:
        let
          hasFrontend = builtins.hasAttr system (konductor.devShells or {})
            && builtins.hasAttr "frontend" (konductor.devShells.${system} or {});
        in
        if hasFrontend then {
          default = konductor.devShells.${system}.frontend;
        } else {
          # Fallback: basic devshell from nixpkgs
          default = let
            pkgs = nixpkgs.legacyPackages.${system};
          in pkgs.mkShell {
            buildInputs = with pkgs; [
              rustup
              nodejs_22
              nodePackages.pnpm
              pkg-config
              openssl
            ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              webkitgtk_4_1
              gtk3
              libsoup_3
              glib-networking
            ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              darwin.apple_sdk.frameworks.WebKit
              darwin.apple_sdk.frameworks.AppKit
            ];
          };
        }
      );
    };
}
