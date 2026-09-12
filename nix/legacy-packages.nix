flakeInputs:
flakeInputs.nixpkgs.lib.genAttrs
  [
    "x86_64-linux"
    "aarch64-linux"
  ]
  (
    system:
    import flakeInputs.nixpkgs {
      inherit system;
      overlays = [
        flakeInputs.self.overlays.default
      ];
    }
  )
