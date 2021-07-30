{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    buildInputs = with pkgs; [
      cargo
      clippy
    ];
    shellHook = ''
      export CELESTIUM_DATA_DIR=./data
      export CELESTIUM_EZ_MODE=yep
    '';
}
