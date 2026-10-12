{
  lib,
  rustPlatform,
  makeWrapper,
  ffmpeg,
  flac,
  mkvtoolnix,
  cacert,
}:

rustPlatform.buildRustPackage {
  pname = "muxcraft";
  version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
  src = builtins.path {
    path = ./.;
    name = "muxcraft-source";
    filter = path: type:
      let
        root = toString ./.;
        relative = lib.removePrefix "${root}/" (toString path);
      in
      path == root
      || builtins.elem relative [ "Cargo.toml" "Cargo.lock" "src" "tests" ]
      || lib.hasPrefix "src/" relative
      || lib.hasPrefix "tests/" relative;
  };
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [ makeWrapper ];
  nativeCheckInputs = [ ffmpeg flac mkvtoolnix ];
  SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";

  postInstall = ''
    wrapProgram $out/bin/muxcraft \
      --prefix PATH : ${lib.makeBinPath [ ffmpeg flac mkvtoolnix ]} \
      --set SSL_CERT_FILE ${cacert}/etc/ssl/certs/ca-bundle.crt
  '';

  meta = {
    description = "Auditable batch remuxing driven by TOML";
    homepage = "https://github.com/IncYami/muxcraft";
    license = lib.licenses.mpl20;
    mainProgram = "muxcraft";
    platforms = [ "x86_64-linux" "aarch64-linux" ];
  };
}
