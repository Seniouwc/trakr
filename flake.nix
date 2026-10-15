{
  description = "trakr dev shell";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      # Build AND run inside this shell (`nix develop`), and `cargo clean` after changing nixpkgs: the binary
      # records the store paths of the libraries it was linked against, and mixing libraries from another
      # nixpkgs revision gives "version `GLIBC_2.xx' not found" errors at start-up.
      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [ rustc cargo clippy rustfmt rust-analyzer pkg-config cmake ];
        buildInputs = with pkgs; [ alsa-lib libopus ];
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        CMAKE_POLICY_VERSION_MINIMUM = "3.5";
        # the audio/GUI libraries are loaded at run time by cpal/winit/eframe
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
          alsa-lib libopus libGL libxkbcommon wayland xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr
        ]);
      };
    };
}
