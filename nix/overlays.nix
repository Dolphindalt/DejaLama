flakeInputs:
let
  inherit (flakeInputs.nixpkgs.lib)
    composeManyExtensions
    ;

  top-level = final: prev: {
    cargo-nice-plug = final.callPackage ./cargo-nice-plug.nix { };
  };
in
{
  default = composeManyExtensions([
    top-level
  ]);
}
