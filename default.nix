{ pkgs ? import <nixpkgs> { }, revision ? null }:

pkgs.callPackage ./package.nix { inherit revision; }
