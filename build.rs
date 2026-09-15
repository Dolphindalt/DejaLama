//! With the `embed-assets` feature, write the original's bitmaps to `assets/*.png` for the editor
//! to embed (see `src/original.rs` for where the DLL comes from). Without it, the plugin fetches
//! them at first launch and this script does nothing.

#[path = "src/original.rs"]
#[allow(dead_code)] // the plugin uses the rest of the module
mod original;

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    println!("cargo::rerun-if-env-changed=DEJA_LAMA_DLL");
    println!("cargo::rerun-if-env-changed=CARGO_FEATURE_EMBED_ASSETS");
    if std::env::var_os("CARGO_FEATURE_EMBED_ASSETS").is_none() {
        return ExitCode::SUCCESS;
    }
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let assets = Path::new(&manifest_dir).join("assets");
    for (_, name, _) in original::BITMAPS {
        println!(
            "cargo::rerun-if-changed={}",
            original::bitmap_path(&assets, name).display()
        );
    }
    if !assets.join(original::PACKAGE_NAME).exists() {
        println!(
            "cargo::warning=downloading {} into assets/",
            original::PACKAGE_URL
        );
    }
    match original::ensure_bitmaps(&assets) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "The editor's bitmaps come from the original plugin. {} Then build again.",
                original::HINT
            );
            ExitCode::FAILURE
        }
    }
}
