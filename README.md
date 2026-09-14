# Deja Lama

Deja Lama is the singing monk: an unofficial one-to-one re-creation of AudioNerdz' Delay Lama (2002) as a CLAP and VST3 instrument for current DAWs. Play a note and the monk sings it; move the pad and he changes pitch and vowel. It sounds like the original sample for sample and looks like it pixel for pixel.

## Download

Get the archive for your platform from the [releases page](https://github.com/dolphindalt/DejaLama/releases), unpack it and copy `deja_lama.clap` and `deja_lama.vst3` into your plugin folders (step 6 below), then rescan plugins in your DAW. The bundles are unsigned: on macOS, Gatekeeper blocks them once, so remove the quarantine flag (`xattr -dr com.apple.quarantine <bundle>`) or allow them in System Settings; on Windows, SmartScreen warns once. The first time the editor opens, the plugin fetches the original's bitmaps as described in step 5.

## Build from source

1. Install Rust from <https://rustup.rs> (1.95 or newer). On macOS, install the Xcode command line tools first (`xcode-select --install`); on Windows, rustup asks for the Visual Studio C++ build tools.
2. On Debian and Ubuntu, install the linker and `curl`: `sudo apt install build-essential curl`. Nix users skip steps 1 to 3: `nix develop` provides everything.
3. Install the plugin bundler: `cargo install cargo-nice-plug`.
4. Get the source: `git clone https://github.com/dolphindalt/DejaLama`, then open a terminal in the new folder; run every command below from there.
5. Build the plugin:

   ```sh
   cargo nice-plug bundle deja_lama --release
   ```

   The first build compiles the dependencies (a few minutes). The bundle carries none of the original's bitmaps: the first time you open the editor, the plugin fetches the original Delay Lama package (1.3 MB) from the Internet Archive with `curl` and keeps the bitmaps in your data folder (`~/.local/share/deja-lama` on Linux, `~/Library/Application Support/Deja Lama` on macOS, `%APPDATA%\Deja Lama` on Windows). Without network access, put `Delay Lama.zip` or `Delay Lama.dll` into that folder yourself, or set `DEJA_LAMA_DLL` to the DLL's path; `DEJA_LAMA_ASSETS` moves the folder. To bake the bitmaps into the bundle instead, add `--features embed-assets`: the build then fetches the package into `assets/` the same way.

6. Copy the bundles from `target/bundled/` into your plugin folders. On Linux:

   ```sh
   mkdir -p ~/.clap ~/.vst3
   cp -r target/bundled/deja_lama.clap ~/.clap/
   cp -r target/bundled/deja_lama.vst3 ~/.vst3/
   ```

   On macOS use `~/Library/Audio/Plug-Ins/CLAP/` and `~/Library/Audio/Plug-Ins/VST3/`, on Windows `C:\Program Files\Common Files\CLAP\` and `C:\Program Files\Common Files\VST3\`.

7. Rescan plugins in your DAW and add "Deja Lama" as an instrument.

The Linux build works; the macOS and Windows builds lack testing.

## Play

Play MIDI notes 16 to 84 to make the monk sing. He sings one note at a time, an octave below the key, and glides when you hold a second key. The editor works like the original: the Glide and Voice knobs, the Delay fader, the pad (press and drag to sing a pitch along X and a vowel along Y) and the "?" button.

From MIDI: pitch bend picks the vowel, CC1 the vibrato, CC5 the glide time, CC7 the volume, CC11 the pitch, CC12 the delay, CC13 the head size.

## Presets

The original's five programs (Rabten, Dorje, Ngawang, Jamyang, Tinley) ship as VST3 presets in `presets/`. Copy them where your DAW looks for presets of the vendor `dcaron` and the plugin `Deja Lama`; on Linux:

```sh
mkdir -p ~/.vst3/presets/dcaron/Deja\ Lama && cp presets/*.vstpreset ~/.vst3/presets/dcaron/Deja\ Lama/
```

On macOS the folder is `~/Library/Audio/Presets/dcaron/Deja Lama/`, on Windows `Documents\VST3 Presets\dcaron\Deja Lama\`. A preset sets glide, delay and head size and leaves the vowel alone, like the original's program menu. CLAP hosts have no shared preset format.

## Standalone

To play without a DAW:

```sh
cargo run --release --features standalone -- --backend jack
```

`--backend alsa` and `--backend dummy` work as well; `--help` lists the options. This build needs `pkg-config` and the ALSA and JACK development libraries (`pkg-config libasound2-dev libjack-jackd2-dev` on Debian and Ubuntu).

## For developers

`cargo test` replays MIDI scripts through the engine and checks the output's hash against the original's, then runs the unit tests. `AGENTS.md` describes the layout and the rules that keep the sound and the look identical; `CONTRIBUTING.md` says how to submit a change.

## Credits and license

Deja Lama is an unofficial re-creation with no connection to AudioNerdz. The synthesis follows the original's FOF design (formant wave functions, after Xavier Rodet's CHANT at IRCAM). The source carries the MIT license; Steinberg's VST 3 terms apply to the VST3 bundle. The original's bitmaps are part of neither this repository nor the bundles it builds: the plugin fetches them from the original package onto your machine at first launch, and they remain the property of their authors.
