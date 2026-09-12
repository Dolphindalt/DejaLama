//! Build the lookup tables and the timing constants that depend only on the sample rate.

// Keep the truncated pi, 2pi and e literals below: the DLL uses exactly these values, and the
// true constants change table values in the last bit and break sample-exactness.
#![allow(clippy::excessive_precision, clippy::approx_constant)]

use super::x87::{ftol, x87};

/// Sine / LFO table length.
pub(super) const SIN_LEN: usize = 1024;
/// Pitch table length: 128 notes × 32 steps per semitone.
pub(super) const PITCH_LEN: usize = 4096;
/// Formant table length: 4 spline segments × 320 entries.
pub(super) const FORMANT_LEN: usize = 1280;

// Formant frequency knots (Hz) for the 5 vowels along the vowel axis: u, o, a, e, i.
const F1_KNOTS: [i32; 5] = [280, 450, 800, 350, 270];
const F2_KNOTS: [i32; 5] = [600, 800, 1150, 2000, 2140];
const F3_KNOTS: [i32; 5] = [2240, 2830, 2900, 2800, 2950];

// Constants at the precision the DLL uses. Keep the f64 and f32 literals as they are: the
// choice only moves the last bits, but it keeps the tables bit-identical.
const TWO_PI_D: f64 = 6.283_185_307;
const PI_D: f64 = 3.141_592_654;
const E_D: f64 = 2.718_281_828;
const SEMITONE_D: f64 = 1.059_463_094;
const MIDI0_HZ_D: f64 = 8.175_798_916;
const FIXED_HI_PERIOD_S: f32 = 0.000_202_020_208_234_898_750; // 1/4950 Hz (fixed formant "F5")
const FIXED_LO_PERIOD_S: f32 = 0.000_263_157_882_727_682_600; // 1/3800 Hz (fixed formant "F4")
const FIXED_HI_DECAY_STEP: f32 = 3.0; // exp-table steps per sample for 4950 Hz -> BW 150 Hz
const FIXED_LO_DECAY_STEP: f32 = 3.6; // exp-table steps per sample for 3800 Hz -> BW 180 Hz
const BW_TO_STEP: f32 = 0.02; // exp table is exp(-50π t): BW_Hz / 50 steps per sample

/// Interpolate 5 integer knots into 4×320 float entries with a Catmull-Rom spline, like the
/// DLL. Pad the endpoints by duplication: p = {f0,f0,f1,f2,f3,f4,f4}.
fn build_formant_table(knots: &[i32; 5]) -> Box<[f32]> {
    let p = [
        knots[0], knots[0], knots[1], knots[2], knots[3], knots[4], knots[4],
    ];
    let mut out = vec![0.0; FORMANT_LEN].into_boxed_slice();
    for s in 1..=4usize {
        let (p0, p1, p2, p3) = (p[s - 1], p[s], p[s + 1], p[s + 2]);
        // Evaluate in extended precision with a single rounding at the store, like the x87 code.
        let c3 = f64::from(3 * (p1 - p2) - p0 + p3) * 0.5; // 0.5(-p0 + 3p1 - 3p2 + p3)
        // Keep the integer division in (5·p1 + p3)/2: the DLL computes it in integer arithmetic.
        #[allow(clippy::manual_midpoint)]
        // ≈ p0 - 2.5p1 + 2p2 - 0.5p3
        let c2 = f64::from(2 * p2 + p0) - f64::from((5 * p1 + p3) / 2);
        let c1 = f64::from(p2 - p0) * 0.5;
        let c0 = f64::from(p1);
        let base = (s - 1) as f64 * 320.0;
        for (j, entry) in out.iter_mut().enumerate().take(s * 320).skip((s - 1) * 320) {
            let t = (j as f64 - base) * f64::from(0.003_125f32); // /320 (f32 constant)
            *entry = (((c3 * t + c2) * t + c1) * t + c0) as f32;
        }
    }
    out
}

/// Everything that depends only on the sample rate: the lookup tables and the timing constants
/// the DLL derives from it at reset. Build once per sample rate.
pub(super) struct Tables {
    pub(super) sample_rate: f32,
    /// grain length, sr·0.02 (20 ms)
    pub(super) grain_len: usize,
    /// exp(-i·50π/sr), 4·`grain_len` entries
    pub(super) exp: Box<[f32]>,
    /// sin(2πi/1024); the DLL keeps two identical copies
    pub(super) sin: Box<[f32]>,
    /// 1024/sr
    pub(super) sin_inc_per_hz: f32,
    /// sr/1024
    pub(super) lfo_sr_over_len: f32,
    /// fixed formants 4950 Hz (BW 150) and 3800 Hz (BW 180)
    pub(super) breath: Box<[f32]>,
    /// grain window
    pub(super) win: Box<[f32]>,
    /// 8.1758·2^(i/32/12) Hz
    pub(super) pitch: Box<[f32]>,
    /// F1, F2, F3 over the vowel axis
    pub(super) formants: [Box<[f32]>; 3],
    /// exp-table step per sample = BW/50
    pub(super) bw_step: [f32; 3],
    /// vibrato re-randomisation interval, sr·0.104
    pub(super) vib_interval: i32,
    /// smoothing step interval, sr·0.01
    pub(super) smooth_interval: i32,
    /// smoothing steps, (sr·0.1) / (sr·0.01) = 10
    pub(super) smooth_steps: i32,
    /// mouth animation step, sr·0.208
    pub(super) mouth_step: i32,
    /// initial delay read indices, sr·-0.309592 and sr·-0.398435
    pub(super) delay_read_l: i32,
    pub(super) delay_read_r: i32,
}

impl Tables {
    // single-letter names come from the formulas
    #[allow(clippy::many_single_char_names)]
    pub(super) fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let srd = f64::from(sr);
        let grain_len = x87(sr).times(0.02).trunc() as usize;

        // exp[i] = e^(-i·k), k = 50π/sr; step through it at BW/50 entries per sample for
        // exp(-π·BW·t)
        let k = (157.079_632_7f64 / srd) as f32;
        let exp: Box<[f32]> = (0..grain_len * 4)
            .map(|i| E_D.powf(-(i as f64 * f64::from(k))) as f32)
            .collect();

        let sin: Box<[f32]> = (0..SIN_LEN)
            .map(|i| ((i as f64 * TWO_PI_D) / SIN_LEN as f64).sin() as f32)
            .collect();

        // Fixed high formants: 4950 Hz (BW 150) and 3800 Hz (BW 180). Accumulate the decays in
        // extended precision (f32 constants, wide accumulator) exactly like the x87 code.
        let mut breath = vec![0.0; grain_len].into_boxed_slice();
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for (i, entry) in breath.iter_mut().enumerate() {
            let w = i as f64 * TWO_PI_D;
            let s1 = (w / (srd * f64::from(FIXED_HI_PERIOD_S))).sin();
            let v = (s1 * f64::from(exp[ftol(a) as usize])) as f32;
            let s2 = (w / (srd * f64::from(FIXED_LO_PERIOD_S))).sin();
            *entry = (s2 * f64::from(exp[ftol(b) as usize]) + f64::from(v)) as f32;
            a += f64::from(FIXED_HI_DECAY_STEP);
            b += f64::from(FIXED_LO_DECAY_STEP);
        }

        // Grain window: half-cosine attack over 1.8 ms, unity, then half-cosine decay.
        // Evaluate the decay as cos(π(decay_len + i)/decay_len) = -cos(πi/decay_len) with the
        // absolute index i (not i - decay_start), exactly like the DLL: the fall therefore
        // begins at i = 2·decay_len and ends near 0.05 rather than 0.
        let attack_len = x87(sr).times(0.0018).trunc();
        let decay_start = x87(sr).times(0.013).trunc();
        let decay_len = x87(sr).times(0.007).trunc();
        let mut win = vec![1.0f32; grain_len].into_boxed_slice();
        for (i, entry) in win.iter_mut().enumerate().take(attack_len.max(0) as usize) {
            *entry = ((1.0 - ((i as f64 * PI_D) / f64::from(attack_len)).cos()) * 0.5) as f32;
        }
        for (i, entry) in win.iter_mut().enumerate().skip(decay_start.max(0) as usize) {
            let v = ((f64::from(decay_len + i as i32) * PI_D) / f64::from(decay_len)).cos();
            *entry = ((1.0 - v) * 0.5) as f32;
        }

        // Pitch table: 1/32 semitone resolution, MIDI note 0 = 8.1758 Hz.
        let pitch = (0..PITCH_LEN)
            .map(|i| (SEMITONE_D.powf(i as f64 * 0.03125) * MIDI0_HZ_D) as f32)
            .collect();

        Self {
            sample_rate,
            grain_len,
            exp,
            sin,
            sin_inc_per_hz: (SIN_LEN as f64 / srd) as f32,
            lfo_sr_over_len: (srd / SIN_LEN as f64) as f32,
            breath,
            win,
            pitch,
            formants: [
                build_formant_table(&F1_KNOTS),
                build_formant_table(&F2_KNOTS),
                build_formant_table(&F3_KNOTS),
            ],
            bw_step: [
                32.5f32 * BW_TO_STEP,
                47.5f32 * BW_TO_STEP,
                62.5f32 * BW_TO_STEP,
            ],
            vib_interval: x87(sr).times(104.0f32).times(0.001).trunc(),
            smooth_interval: x87(sr).times(0.01f32).trunc(),
            smooth_steps: x87(sr).times(0.001f32).times(100.0f32).trunc()
                / x87(sr).times(0.01f32).trunc(),
            mouth_step: x87(sr).times(0.208).trunc(),
            delay_read_l: x87(sr).times(-0.309_592).trunc(),
            delay_read_r: x87(sr).times(-0.398_435).trunc(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values on purpose
mod tests {
    use super::*;

    /// Pin the DLL's sample-rate-derived sizes, including the ones the x87 rounding moves off
    /// the naive value: `smooth_interval` at 44.1 kHz is 440 (44100 · f32(0.01) < 441) and the
    /// decay start at 96 kHz is 1247 (96000 · f64(0.013) < 1248).
    #[test]
    fn sizes_follow_the_dll() {
        // (rate, grain_len, smooth_interval, vib_interval, mouth_step, delay_read_l, delay_read_r)
        let expected = [
            (22050.0, 441, 220, 2293, 4586, -6826, -8785),
            (44100.0, 882, 440, 4586, 9172, -13653, -17570),
            (48000.0, 960, 479, 4992, 9983, -14860, -19124),
            (88200.0, 1764, 881, 9172, 18345, -27306, -35141),
            (96000.0, 1920, 959, 9984, 19967, -29720, -38249),
            (176_400.0, 3528, 1763, 18345, 36691, -54612, -70283),
            (192_000.0, 3840, 1919, 19968, 39935, -59441, -76499),
        ];
        for (sr, grain_len, smooth, vib, mouth, read_l, read_r) in expected {
            let t = Tables::new(sr);
            let got = (
                sr,
                t.grain_len,
                t.smooth_interval,
                t.vib_interval,
                t.mouth_step,
                t.delay_read_l,
                t.delay_read_r,
            );
            assert_eq!(got, (sr, grain_len, smooth, vib, mouth, read_l, read_r));
            assert_eq!(t.smooth_steps, 10, "{sr} Hz");
            assert_eq!(t.exp.len(), 4 * grain_len);
            assert_eq!((t.breath.len(), t.win.len()), (grain_len, grain_len));
        }
        assert_eq!(x87(96000.0f32).times(0.013).trunc(), 1247);
    }

    /// Keep the window's absolute-index quirk: the fall starts at 2 · `decay_len`, not at the
    /// decay start, and the last sample ends near 0.05 rather than 0.
    #[test]
    fn window_keeps_the_absolute_index_quirk() {
        let sr = 44100.0f32;
        let t = Tables::new(sr);
        let attack_len = x87(sr).times(0.0018).trunc() as usize;
        let decay_start = x87(sr).times(0.013).trunc() as usize;
        let decay_len = x87(sr).times(0.007).trunc() as usize;
        assert_eq!((attack_len, decay_start, decay_len), (79, 573, 308));
        assert_eq!(t.win[0], 0.0);
        assert!(t.win[attack_len..decay_start].iter().all(|&w| w == 1.0));
        assert!((0.94..0.96).contains(&t.win[decay_start]));
        assert_eq!(t.win[2 * decay_len], 1.0);
        assert!((0.04..0.06).contains(&t.win[t.grain_len - 1]));
    }

    /// Check that the spline passes through the knots at the segment boundaries.
    #[test]
    fn formant_tables_hit_the_knots() {
        let t = Tables::new(44100.0);
        for (table, knots) in t.formants.iter().zip([F1_KNOTS, F2_KNOTS, F3_KNOTS]) {
            assert_eq!(table.len(), FORMANT_LEN);
            for (segment, &knot) in knots.iter().enumerate().take(4) {
                assert_eq!(table[segment * 320], knot as f32);
            }
            // the last knot sits one entry past the table; the curve ends just short of it
            assert!((table[FORMANT_LEN - 1] - knots[4] as f32).abs() < 0.5);
        }
    }

    #[test]
    fn pitch_table_is_in_tune() {
        let t = Tables::new(44100.0);
        assert_eq!(t.pitch.len(), PITCH_LEN);
        assert_eq!(t.pitch[0], 8.175_799);
        assert_eq!(t.pitch[69 * 32], 440.0);
        assert_eq!(t.bw_step, [0.65, 0.95, 1.25]);
    }
}
