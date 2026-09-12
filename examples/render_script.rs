//! Render a script through the engine, for comparisons with the original DLL and for new
//! regression entries.
//!
//! ```sh
//! cargo run --release --example render_script -- tests/scripts/note_on_off_44k.txt
//! cargo run --release --example render_script -- script.txt out.f32
//! ```
//!
//! With one argument, print the frame count and the FNV-1a hash that
//! `tests/engine_regression.rs` pins. With two, write the interleaved little-endian f32 stereo
//! output to the second path, the format the VST2 host writes for the DLL.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (script_path, out_path) = match args.as_slice() {
        [script] => (script, None),
        [script, out] => (script, Some(out)),
        _ => {
            eprintln!("usage: render_script <script.txt> [out.f32]");
            return ExitCode::FAILURE;
        }
    };
    let script = match std::fs::read_to_string(script_path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("{script_path}: {err}");
            return ExitCode::FAILURE;
        }
    };
    let samples = match deja_lama::script::run(&script) {
        Ok(samples) => samples,
        Err(err) => {
            eprintln!("{script_path}: {err}");
            return ExitCode::FAILURE;
        }
    };
    match out_path {
        Some(out) => {
            let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
            if let Err(err) = std::fs::write(out, bytes) {
                eprintln!("{out}: {err}");
                return ExitCode::FAILURE;
            }
        }
        None => println!(
            "{} frames, fnv1a {:#018x}",
            samples.len() / 2,
            deja_lama::script::fnv1a(&samples)
        ),
    }
    ExitCode::SUCCESS
}
