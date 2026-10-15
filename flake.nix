{
  description = "trakr: a tracker DAW";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      lib = pkgs.lib;

      # Tools needed to compile the vendored C encoders (Opus uses the system libopus via pkg-config;
      # LAME and fdk-aac build from source with cmake / autotools).
      buildTools = with pkgs; [ pkg-config cmake autoconf automake libtool gnumake ];
      libs = with pkgs; [ alsa-lib libopus ];
      # Loaded at run time by cpal / winit / eframe.
      runtimeLibs = with pkgs; [ alsa-lib libopus libGL libxkbcommon wayland xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr ];
    in {
      # nix build        -> ./result/bin/trakr (a self-contained wrapper that finds its libraries)
      # nix run          -> build and start it
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "trakr";
        version = "0.0.1";
        src = lib.cleanSource ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = [ "-p" "trakr" ];
        # the workspace tests need audio hardware, python and a lot of build time; run them with cargo
        doCheck = false;
        dontUseCmakeConfigure = true; # cmake is only for the crates' build scripts
        nativeBuildInputs = buildTools ++ [ pkgs.makeWrapper ];
        buildInputs = libs;
        CMAKE_POLICY_VERSION_MINIMUM = "3.5";
        postFixup = ''
          wrapProgram $out/bin/trakr --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
        '';
        meta = {
          description = "Open-source tracker DAW";
          mainProgram = "trakr";
        };
      };
      apps.${system}.default = {
        type = "app";
        program = "${self.packages.${system}.default}/bin/trakr";
      };

      # nix develop: the same dependencies for working with cargo directly. Build and run inside this shell,
      # and `cargo clean` after changing nixpkgs: a binary records the store paths of the libraries it was
      # linked against, and libraries from another nixpkgs revision give "GLIBC_2.xx not found" at start-up.
      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = buildTools ++ (with pkgs; [ rustc cargo clippy rustfmt rust-analyzer ]);
        buildInputs = libs;
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        CMAKE_POLICY_VERSION_MINIMUM = "3.5";
        LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
      };
    };
}
