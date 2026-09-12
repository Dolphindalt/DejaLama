//! Modulate the pitch with the DLL's re-randomised sine LFO.

use super::tables::{SIN_LEN, Tables};
use super::x87::ftol;

/// Sine LFO with a depth of 0.2 + mod wheel semitones and a rate of 4..6 Hz, re-randomised every
/// 104 ms; the mod wheel also speeds the rate up by up to 20%.
pub(super) struct Vibrato {
    /// LFO phase 0..1024
    phase: f32,
    /// rate in Hz
    rate: f32,
    /// samples since the last re-randomisation, and the interval between two
    timer: i32,
    interval: i32,
    /// LCG state (the DLL never initialises it)
    rng_seed: i32,
}

/// The LCG output scale, 2^-32 (exact in f32).
const RNG_SCALE: f32 = 1.0 / 4_294_967_296.0;

impl Vibrato {
    pub(super) const fn new() -> Self {
        Self {
            phase: 0.0,
            rate: 4.0,
            timer: 0,
            interval: 0,
            rng_seed: 0,
        }
    }

    pub(super) fn reset(&mut self, tables: &Tables) {
        self.phase = 0.0;
        self.rate = 4.0;
        self.timer = 0;
        self.interval = tables.vib_interval;
    }

    /// Knuth/NR LCG with a result in [-0.5, 0.5), the DLL's random generator. Return it in
    /// extended precision: the DLL keeps it on the FPU stack and never rounds it to float.
    fn random(&mut self) -> f64 {
        self.rng_seed = self
            .rng_seed
            .wrapping_mul(0x0019_660d)
            .wrapping_add(0x3c6e_f35f);
        f64::from(self.rng_seed) * f64::from(RNG_SCALE)
    }

    /// Advance one gated sample and return the vibrato in semitones.
    pub(super) fn tick(&mut self, mod_wheel: f32, tables: &Tables) -> f32 {
        if self.phase >= SIN_LEN as f32 {
            self.phase -= SIN_LEN as f32;
        }
        if self.timer >= self.interval {
            self.timer = 0;
            let r = self.random();
            self.rate = ((r + r) + f64::from(5.0f32)) as f32;
        }
        let out = ((f64::from(mod_wheel) + f64::from(0.2f32))
            * f64::from(tables.sin[ftol(f64::from(self.phase)) as usize])) as f32;
        self.phase = (((f64::from(mod_wheel) * f64::from(0.2f32) + 1.0) * f64::from(self.rate))
            / f64::from(tables.lfo_sr_over_len)
            + f64::from(self.phase)) as f32;
        out
    }

    /// Count one sample, gated or not. Wrap like the DLL's `int`: the timer only resets while
    /// a key is down, so an idle voice counts past `i32::MAX` after 2^31 samples.
    pub(super) fn advance(&mut self) {
        self.timer = self.timer.wrapping_add(1);
    }
}
