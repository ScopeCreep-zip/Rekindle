{
  lib,
  rustPlatform,
  cargo-tauri,
  pkg-config,
  nodejs,
  pnpm_10,
  fetchPnpmDeps,
  pnpmConfigHook,
  capnproto,
  cmake,
  libsodium,
  openssl,
  webkitgtk_4_1,
  libsoup_3,
  glib-networking,
  alsa-lib,
  libopus,
  libayatana-appindicator,
  wrapGAppsHook4,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../src-tauri/Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "rekindle";
  version = cargoToml.package.version;

  src = lib.fileset.toSource {
    root = ./..;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src-tauri
      ../crates
      ../schemas
      ../src
      ../package.json
      ../pnpm-lock.yaml
      ../tsconfig.json
      ../tsconfig.node.json
      ../vite.config.ts
      ../index.html
    ];
  };

  buildAndTestSubdir = "src-tauri";

  cargoLock.lockFile = ../Cargo.lock;

  pnpmDeps = fetchPnpmDeps {
    pname = "rekindle";
    version = cargoToml.package.version;
    src = lib.fileset.toSource {
      root = ./..;
      fileset = lib.fileset.unions [
        ../package.json
        ../pnpm-lock.yaml
      ];
    };
    pnpm = pnpm_10;
    fetcherVersion = 1;
    hash = "sha256-/ITBLe8G9eXnsPGz/T5SMefuAjH3C68R2RYApInik1E=";
  };

  nativeBuildInputs = [
    cargo-tauri.hook
    nodejs
    pnpm_10
    pnpmConfigHook
    pkg-config
    capnproto
    cmake
    wrapGAppsHook4
  ];

  buildInputs = [
    openssl
    webkitgtk_4_1
    libsoup_3
    glib-networking
    libsodium
    alsa-lib
    libopus
    libayatana-appindicator
  ];

  env = {
    SODIUM_USE_PKG_CONFIG = "1";
    OPENSSL_NO_VENDOR = "1";
  };

  postPatch = ''
    substituteInPlace src-tauri/tauri.conf.json \
      --replace-fail '"targets": "all"' '"targets": ["deb"]'

    # libappindicator-sys uses dlopen at runtime — patch absolute path
    substituteInPlace $cargoDepsCopy/libappindicator-sys-*/src/lib.rs \
      --replace-fail "libayatana-appindicator3.so.1" "${libayatana-appindicator}/lib/libayatana-appindicator3.so.1"
  '';

  meta = with lib; {
    description = "Xfire rebuilt as a decentralized Tauri 2 app";
    homepage = "https://github.com/ScopeCreep-zip/rekindle";
    license = licenses.mit;
    platforms = platforms.linux;
    mainProgram = "rekindle";
  };
}
