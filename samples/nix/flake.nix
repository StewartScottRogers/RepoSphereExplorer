{
  description = "csvstats: summary statistics for a column of readings";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        toolchain = pkgs.rust-bin.stable.latest.default;
      in
      {
        packages.default = pkgs.callPackage ./default.nix { };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            toolchain
            pkgs.cargo-audit
            pkgs.just
          ];

          shellHook = ''
            echo "csvstats development shell"
          '';
        };

        checks.format = pkgs.runCommand "check-format" { } ''
          ${toolchain}/bin/cargo fmt --manifest-path ${./.}/Cargo.toml -- --check
          touch $out
        '';
      });
}
