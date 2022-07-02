{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk/master";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, utils, naersk }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        naersk-lib = pkgs.callPackage naersk { };
        wgsl-analyzer = with pkgs; stdenv.mkDerivation rec {
          pname = "wgsl-analyzer";
          version = "0.4.5";
          src = fetchurl {
            url = "https://github.com/wgsl-analyzer/wgsl-analyzer/releases/download/v${version}/wgsl_analyzer-linux-x64";
            sha256 = "4e1fc47836d3f08778171181b06e82ccbc766746b3f6859cb9b13145355e2834";
          };
          nativeBuildInputs = [ autoPatchelfHook ];
          dontUnpack = true;
          installPhase = ''
            install -m755 -D $src $out/bin/wgsl_analyzer
          '';
          meta = {
            homepage = "https://github.com/wgsl-analyzer/wgsl-analyzer";
            description = "A language server implementation for the WGSL shading language";
          };
        };
        native-deps = with pkgs; [
          pkgconfig
          patchelf
        ];
        deps = (with pkgs; [
          wayland
          libGL
          vulkan-headers
          vulkan-loader
          vulkan-tools
          vulkan-validation-layers
        ]) ++ (with pkgs; with pkgs.xorg; [
          libX11
          libxcb
          libxkbcommon
          xcbutil
          xcbutilkeysyms
          xcbutilwm
          libXcursor
          libXrandr
          libXi
        ]);
        dev-deps = with pkgs; [
          cargo
          rustc
          rustfmt
          rustPackages.clippy

          rust-analyzer
          taplo-cli
          wgsl-analyzer
          rnix-lsp
        ];
      in
      {
        defaultPackage = naersk-lib.buildPackage {
          src = ./.;
          buildInputs = deps;
          nativeBuildInputs = native-deps;
          postInstall = ''
            patchelf --add-needed "${pkgs.vulkan-loader}/lib/libvulkan.so.1" $out/bin/wgpu-block-engine

          '';
        };

        defaultApp = utils.lib.mkApp {
          drv = self.defaultPackage."${system}";
        };

        devShell = with pkgs; mkShell {
          buildInputs = deps ++ native-deps ++ dev-deps;
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
          LD_LIBRARY_PATH = pkgs.lib.concatStringsSep ":" (map (p: "${p}/lib") [
            vulkan-loader
          ]);
          VK_LAYER_PATH = "${pkgs.vulkan-validation-layers}/share/vulkan/explicit_layer.d";
        };
      });
}
