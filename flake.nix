{
  description = "RMK firmware for the ZSA Voyager (GD32F303) - dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rustfmt"
            "llvm-tools"
            "clippy"
          ];
          targets = [ "thumbv7em-none-eabihf" ];
        };

        # flip-link wrapped so rust-lld is on PATH if .cargo/config.toml
        # ever wires it as the linker.
        flipLink = pkgs.symlinkJoin {
          name = "flip-link-wrapped";
          paths = [ pkgs.flip-link ];
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postBuild = ''
            wrapProgram $out/bin/flip-link \
              --prefix PATH : ${rustToolchain}/lib/rustlib/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/bin
          '';
        };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            flipLink
            pkgs.cargo-binutils
            pkgs.dfu-util
            pkgs.stdenv.cc
          ];

          # rmk-types trips a rustc stack overflow during monomorphization
          # without a larger stack. Mirrors .cargo/config.toml's [env] so
          # the value is right even if cargo is invoked via a wrapper that
          # bypasses that file.
          RUST_MIN_STACK = "16777216";
        };
      }
    );
}
