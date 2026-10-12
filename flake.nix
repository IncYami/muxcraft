{
  description = "Auditable batch remuxing driven by TOML";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          muxcraft = pkgs.callPackage ./package.nix { };
        in {
          default = muxcraft;
          muxcraft = muxcraft;
        });
    in {
      packages = packages;
      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${packages.${system}.default}/bin/muxcraft";
          meta.description = "Run the muxcraft command line application";
        };
      });
    };
}
