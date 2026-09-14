flakeInputs:
flakeInputs.nixpkgs.lib.genAttrs
  [
    "x86_64-linux"
    "aarch64-linux"
    "x86_64-darwin"
    "aarch64-darwin"
  ]
  (
    system:
    let
      pkgs = flakeInputs.self.legacyPackages.${system};
      # On Linux, the libraries the plugin (baseview/egui_glow) dlopens at run time and the ones
      # the standalone (cpal/jack) links against; macOS links the system frameworks instead.
      runtimeLibs = pkgs.lib.optionals pkgs.stdenv.isLinux (
        with pkgs;
        [
          libGL
          libx11
          libxcursor
          libxrandr
          libxi
          libxcb
          libxkbcommon
          alsa-lib
          jack2
        ]
      );
    in
    {
      default = pkgs.mkShell {
        packages = with pkgs; [
          rustc
          cargo
          cargo-nice-plug # `cargo nice-plug bundle`; packaged in ./cargo-nice-plug.nix
          rustfmt
          clippy
          rust-analyzer
          pkg-config # alsa-sys (standalone feature) locates alsa-lib through it
          curl # the plugin and build.rs download the original package for the editor's bitmaps
        ];
        buildInputs = runtimeLibs;
        LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.isLinux (pkgs.lib.makeLibraryPath runtimeLibs);
      };
    }
  )
