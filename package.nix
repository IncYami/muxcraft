{
  lib,
  rustPlatform,
  makeWrapper,
  ffmpeg,
  flac,
  mkvtoolnix,
  cacert,
  revision ? null,
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
      || builtins.elem relative [ "Cargo.toml" "Cargo.lock" "build.rs" "src" "tests" "docs" ]
      || lib.hasPrefix "src/" relative
      || lib.hasPrefix "tests/" relative
      || lib.hasPrefix "docs/" relative;
  };
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [ makeWrapper ];
  nativeCheckInputs = [ ffmpeg flac mkvtoolnix ];
  SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
  MUXCRAFT_BUILD_SHA = if revision == null then "unknown" else builtins.substring 0 8 revision;

  postInstall = ''
    wrapProgram $out/bin/muxcraft \
      --prefix PATH : ${lib.makeBinPath [ ffmpeg flac mkvtoolnix ]} \
      --set SSL_CERT_FILE ${cacert}/etc/ssl/certs/ca-bundle.crt
  '';

  meta = {
    description = "Auditable batch remuxing driven by TOML";
    homepage = "https://github.com/IncYami/muxcraft";
    license = [ lib.licenses.mpl20 lib.licenses.bsd3 ];
    mainProgram = "muxcraft";
    platforms = [ "x86_64-linux" "aarch64-linux" ];
  };
}
