{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "df";
  version = "0.1.0";

  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;

  meta = {
    description = "AST-based code clone detector";
    mainProgram = "df";
    platforms = lib.platforms.unix;
  };
}
