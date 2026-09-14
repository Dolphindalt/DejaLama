# Contributing

Read `AGENTS.md` first: it lists the layout and the rules, and the rules are strict. The engine mirrors the original plugin sample for sample and the editor mirrors its window pixel for pixel, so most changes that alter either are unwanted, however sensible they look.

Before you open a pull request, run `cargo fmt`, `cargo clippy --all-targets`, and `cargo test`, with and without `--features embed-assets`; CI runs the same on Linux, macOS and Windows. A change that makes a hash test fail changes the sound. If you intend that, render the affected scripts with `cargo run --release --example render_script -- <script>` and with the original plugin, compare both outputs sample for sample, and put the new hashes and the reason in the pull request. The VST2 host that replays a script through the original is not part of this repository.

Never add the original's bitmaps, package or DLL to a commit. The plugin (or `build.rs` with `--features embed-assets`) fetches them onto your machine, and `.gitignore` keeps them out.
