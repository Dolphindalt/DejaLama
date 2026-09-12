# Deja Lama agent notes

Read this file before you change anything in this repository.

Deja Lama re-creates AudioNerdz' Delay Lama (2002), the VST2 "virtual singing monk" (FOF vocal synthesis), as a CLAP/VST3 instrument on nice-plug (a nih-plug fork) with an egui editor. Match the original `Delay Lama.dll` bit for bit in the engine and pixel for pixel in the editor.

## Layout

| path | contents |
|---|---|
| `src/engine.rs` | `DelayLama`: owns the parts below, runs the per-sample control loop and the output pass. No framework dependencies. |
| `src/engine/` | One concern per file: `x87` (FPU rounding emulation), `tables`, `grains` (FOF grain and overlap-add ring), `notes` (held-key stack), `glide`, `vibrato`, `smoothers` (pitch and vowel ramps), `mouth` (editor animation), `delay`, `midi` (messages and the bend-spreading workaround), `params` (parameters and factory programs). |
| `src/lib.rs` | nice-plug glue: `DejaLama`, `DejaLamaParams`, MIDI and parameter translation, CLAP/VST3 export. |
| `src/gui.rs` | Editor: draws the DLL's bitmaps and re-implements its VSTGUI 2.2 controls. Geometry table at the top. |
| `src/shared.rs` | Atomics that carry mouth, vowel and expression from the audio thread to the editor. |
| `src/script.rs` | Script interpreter for the regression tests and the render example. The same commands drive the DLL through a VST2 host, so that you can compare both outputs sample for sample. |
| `src/main.rs` | Standalone entry point (feature `standalone`). |
| `build.rs`, `assets/` | Writes the DLL's eight bitmaps to `assets/*.png` (white pixels in the three handle images carry alpha 0); its module doc lists where it takes the DLL from and the hash checks. |
| `examples/render_script.rs` | Render a script: print its frame count and FNV-1a hash, or write the interleaved f32 output. |
| `examples/write_presets.rs`, `presets/` | The five factory programs as VST3 preset files, generated from `engine::PROGRAMS`. Regenerate them when the parameter ids, the programs or `VST3_CLASS_ID` change. |
| `tests/engine_regression.rs`, `tests/scripts/` | Hash tests: each script replays through the engine and its output must hash like the DLL's. Unit tests sit next to the code in `src/`. |

## Build, run, test

```sh
cargo nice-plug bundle deja_lama --release   # target/bundled/deja_lama.clap and deja_lama.vst3
mkdir -p ~/.clap ~/.vst3 && cp -r target/bundled/deja_lama.clap ~/.clap/ && cp -r target/bundled/deja_lama.vst3 ~/.vst3/
cargo run --release --features standalone -- --backend jack   # or alsa / dummy; --help lists the options
cargo test                                   # 10 hash tests plus the unit tests; keep them green
cargo run --release --example render_script -- tests/scripts/note_on_off_44k.txt   # frame count and hash
cargo run --example write_presets            # regenerate presets/*.vstpreset
cargo clippy --all-targets ; cargo fmt
```

- `cargo clippy` runs at the pedantic level through the `[lints]` table; keep `suboptimal_flops` and `imprecise_flops` off, because they rewrite float arithmetic.
- `Cargo.toml` explains its lint allowances, the `standalone` gate and the `bench` profile; keep all three as they are.
- `nix develop` provides the toolchain, `cargo-nice-plug`, `curl` and the GL, X11, ALSA and JACK libraries. Keep the nix toolchain and a system toolchain in separate target directories (`CARGO_TARGET_DIR`); mixing them in one `target/` breaks with glibc symbol errors, and `cargo clean` fixes it.
- CI (`.github/workflows/ci.yml`) runs every step inside `nix develop`: fmt, clippy with `-D warnings`, the tests, a standalone check and the bundle. It fetches the original package into `assets/` first and caches it, the Nix store and the cargo registry between runs, and it fails when `assets/` tracks anything beyond `.gitkeep`. Keep every step green.

## Invariants

1. Treat the engine numerics as frozen: truncating float-to-int casts (`ftol`), `f32` storage with wider intermediates, the `X87` type for products the DLL rounds to a 64-bit mantissa, and truncated constants such as `6.283185307` and `2.718281828`. Any change that alters a sample fails `cargo test`. When you intend one, render the test scripts with `examples/render_script.rs` and with the DLL, compare them sample for sample, and update the hashes in `tests/engine_regression.rs`. The VST2 host that replays the scripts through the DLL is not part of this repository.
2. Never allocate on the audio thread. Create every buffer in the engine's `reset()`, which `activate()` reaches through `set_sample_rate()`, and let `process()` only fill them. The MIDI scratch `Vec` and the engine's `midi_out` have fixed capacities and drop what does not fit.
3. Keep blocks at or below `DelayLama::max_block_len()` (the 10240-sample ring minus one grain) and sample rates at or above 1 kHz; `activate()` refuses the rest, and the glue clamps host values to 0..1 before they reach the engine.
4. Measure GUI geometry; never design it. Keep every rect and offset in `gui.rs` as template matching against captures of the DLL's window produced them, the window at 360x510, and oddities such as the fader handle at the top of its rect. Keep the control arithmetic (`knob_value_from_point`, `scaled`, the frame index) in f64 on float constants with a float store at the end: plain f32 picks a different knob frame at the ends of the range.
5. Keep every texture at or below 2048 px per side: hosts report that limit.
6. List every deviation from the DLL here. Current deviations: pad moves reach the host as automatable parameters instead of the MIDI note 40, CC11 and pitch bend the DLL sent; the DLL's hack that spreads pitch bends arriving at offset 0 over the block stays off; the knobs follow the host parameters, while the DLL also moved them from CC5, CC12 and CC13; the knob mode stays circular, which VST2 hosts could switch; a wildcard note off releases every key, which the DLL had no message for; sample-accurate automation splits a host block at parameter changes, so the per-block output gain follows the glide per sub-block.
7. Never commit the original's bitmaps or package. Keep `assets/` empty in git apart from `.gitkeep`, and keep the history free of them.

## Parameters and control

| id | name | meaning |
|---|---|---|
| `porttime` | PortTime ("Glide") | legato portamento: 12 semitones per (v + 0.01) s |
| `vowel` | Vowel | 0 u, 0.25 o, 0.5 a, 0.75 e, 1 i; pitch bend also drives it |
| `delay` | Delay | stereo feedback delay level (309.6 and 398.4 ms, feedback 0.5) |
| `headsize` | HeadSize ("Voice") | formant scale 0.75 + 0.5 v |
| `padgate`, `padx`, `pady` | XY pad (hidden in generic UIs) | gate = sing; x sets the pitch (note 36 + 12x, smoothed); y sets the vowel |

MIDI: notes 16 to 84 (monophonic, last-note priority, one octave down), CC1 vibrato depth and rate, CC5 glide, CC7 volume, CC11 pitch 36 to 48, CC12 delay, CC13 head size, pitch bend for the vowel. `engine::PROGRAMS` holds the original's five programs: Rabten's values are the defaults and the script command `program` loads one.
