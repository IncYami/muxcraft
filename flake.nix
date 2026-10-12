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
          muxcraft = pkgs.rustPlatform.buildRustPackage {
            pname = "muxcraft";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            nativeCheckInputs = [ pkgs.ffmpeg pkgs.flac pkgs.mkvtoolnix ];
            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
            postInstall = ''
              wrapProgram $out/bin/muxcraft \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.ffmpeg pkgs.flac pkgs.mkvtoolnix ]} \
                --set SSL_CERT_FILE ${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
            '';
            meta = {
              description = "Auditable batch remuxing driven by TOML";
              homepage = "https://github.com/IncYami/muxcraft";
              license = pkgs.lib.licenses.mpl20;
              mainProgram = "muxcraft";
              platforms = systems;
            };
          };
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
