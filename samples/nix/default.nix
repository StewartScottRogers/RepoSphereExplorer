{ pkgs ? import <nixpkgs> { }
, lib ? pkgs.lib
, stdenv ? pkgs.stdenv
, withDocs ? true
}:

with pkgs;

let
  version = "1.0.3";
  sources = lib.cleanSource ./src;
in
stdenv.mkDerivation rec {
  pname = "csvstats";
  inherit version;

  src = sources;

  nativeBuildInputs = [
    cargo
    rustc
    pkg-config
  ] ++ lib.optional withDocs pandoc;

  buildInputs = [
    openssl
    zlib
  ];

  checkInputs = [
    cargo-nextest
  ];

  doCheck = true;

  buildPhase = ''
    runHook preBuild
    cargo build --release --offline
    runHook postBuild
  '';

  checkPhase = ''
    runHook preCheck
    cargo nextest run --release --offline
    runHook postCheck
  '';

  installPhase = ''
    runHook preInstall
    install -Dm755 target/release/csvstats $out/bin/csvstats
    ${lib.optionalString withDocs ''
      pandoc README.md -o $out/share/doc/csvstats/README.html
    ''}
    runHook postInstall
  '';

  meta = with lib; {
    description = "Summary statistics for a column of readings";
    homepage = "https://example.com/floor/csvstats";
    license = licenses.mit;
    maintainers = [ "the floor" ];
    platforms = platforms.unix;
  };
}
