{
  lib,
  rustPlatform,
  fetchCrate,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "cargo-nice-plug";
  version = "0.1.1";

  src = fetchCrate {
    inherit (finalAttrs) pname version;
    hash = "sha256-QHF9kP1EvSS27IibOddnQADWuXsRKWzvKCvr6kCALJo=";
  };

  cargoHash = "sha256-wT8ghI0rsoeieVY7C41pJcOrC3Lf/ewGtrGxe9bTBzw=";

  meta = {
    description = "Cargo subcommand for compiling and bundling nice-plug audio plugins";
    homepage = "https://codeberg.org/RustAudio/nice-plug";
    license = lib.licenses.isc;
    mainProgram = "cargo-nice-plug";
  };
})
