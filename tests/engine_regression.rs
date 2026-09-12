//! Bit-exactness regression tests for `deja_lama::engine`. Each hash below comes from running the
//! script through the original `Delay Lama.dll` (under Wine, in a minimal VST2 host) and hashing
//! its interleaved f32 stereo output with FNV-1a. Keep the engine reproducing every sample bit
//! for bit. Render a script with `cargo run --release --example render_script -- <script>` to get
//! the frame count and the hash for a new entry.

use deja_lama::script;

fn check(name: &str, script_text: &str, expected: u64, frames: usize) {
    let samples = script::run(script_text).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(samples.len() / 2, frames, "{name}: frame count");
    assert_eq!(
        script::fnv1a(&samples),
        expected,
        "{name}: output differs from the original DLL"
    );
}

/// Declare a bit-exactness test: replay `tests/scripts/<name>.txt` and compare the FNV-1a hash of
/// the output with the DLL's.
macro_rules! hash_test {
    ($name:ident, $hash:literal, $frames:literal) => {
        #[test]
        fn $name() {
            check(
                stringify!($name),
                include_str!(concat!("scripts/", stringify!($name), ".txt")),
                $hash,
                $frames,
            );
        }
    };
}

hash_test!(note_on_off_44k, 0x0d9d_ae5a_ef52_7df5, 76800);
hash_test!(cc_bend_expression_44k, 0x2819_a058_9c29_ff6f, 117_760);
hash_test!(legato_glide_stack_44k, 0xdaa7_7796_56b2_3dcd, 153_600);
hash_test!(gui_params_48k, 0xb47e_c1cb_b05e_9ebf, 104_960);
hash_test!(bend_spread_hack_64blk, 0x7a9e_9c6b_1532_3a39, 17280);
hash_test!(programs_96k, 0x51a6_f627_65aa_e4bf, 163_840);
hash_test!(random_stress_88k_to_44k, 0x52b5_72fd_fce3_9b0c, 384_000);
hash_test!(random_22k_block37, 0x2f2e_cc81_8bb4_7986, 222_000);
hash_test!(random_192k_to_176k, 0x4b6f_6b0b_90bc_9ef9, 1_843_200);
hash_test!(random_48k_resume, 0xdcdf_8baf_cbd3_98eb, 640_000);
