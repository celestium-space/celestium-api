{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    buildInputs = with pkgs; [
      cargo
      clippy
    ];
    shellHook = ''
      export CELESTIUM_DATA_DIR=./data
    '';
}
