{
  description = "Kestrel development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, rust-overlay, ... }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          toolchain = pkgs.rust-bin.stable."1.96.0".default.override {
            extensions = [ "clippy" "rust-analyzer" "rustfmt" ];
          };
          hostXcrun = pkgs.writeShellScriptBin "xcrun" ''
            unset DEVELOPER_DIR
            toolchain=$(/usr/bin/xcodebuild -showComponent MetalToolchain -json 2>/dev/null | /usr/bin/plutil -extract toolchainIdentifier raw -o - - 2>/dev/null || true)
            if [ -n "$toolchain" ]; then
              export TOOLCHAINS="$toolchain"
            fi
            exec /usr/bin/xcrun "$@"
          '';
        in {
          default = pkgs.mkShell {
            packages = [ toolchain pkgs.pkg-config ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ hostXcrun ];
          };
        });
    };
}
