{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  packages = with pkgs; [ cargo rustc rustfmt clippy cargo-audit ffmpeg flac mkvtoolnix ];
}
