{
  description = "Deja Lama, a one-to-one re-creation of Delay Lama as a CLAP/VST3 instrument";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  };

  outputs = flakeInputs: {
    devShells = import ./nix/dev-shells.nix flakeInputs;
    legacyPackages = import ./nix/legacy-packages.nix flakeInputs;
    overlays = import ./nix/overlays.nix flakeInputs;
  };
}
