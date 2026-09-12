//! Build one FOF grain and overlap-add it into the accumulation ring.

use super::tables::{FORMANT_LEN, SIN_LEN, Tables};
use super::x87::{ftol, x87};

/// Length of the grain accumulation ring. Keep it above block size + grain length.
pub const ACC_LEN: usize = 0x2800; // 10240

/// The FOF grain and the overlap-add ring it lands in.
pub(super) struct Grains {
    /// one FOF period; `build` renews it
    pub(super) grain: Box<[f32]>,
    /// overlap-add ring, `ACC_LEN` samples
    acc: Box<[f32]>,
    /// ring position of the last grain start
    write_pos: usize,
    /// ring read position
    read_pos: usize,
}

impl Grains {
    pub(super) fn new(grain_len: usize) -> Self {
        Self {
            grain: vec![0.0; grain_len].into_boxed_slice(),
            acc: vec![0.0; ACC_LEN].into_boxed_slice(),
            write_pos: 0,
            read_pos: 0,
        }
    }

    /// Zero the ring and rewind both positions; keep the grain itself.
    pub(super) fn reset(&mut self) {
        self.acc.fill(0.0);
        self.write_pos = 0;
        self.read_pos = 0;
    }

    /// Build one grain (FOF period) for the vowel and head size.
    pub(super) fn build(&mut self, tables: &Tables, vowel: f32, head_size: f32) {
        let index =
            ftol(f64::from(vowel) * f64::from(1279.0f32)).clamp(0, FORMANT_LEN as i32 - 1) as usize;
        // formant scale 0.75..1.25 from the head size
        let fs = (f64::from(head_size) * 0.5 + 0.75) as f32;
        let inc: [f32; 3] = std::array::from_fn(|k| {
            x87(fs)
                .times(tables.formants[k][index])
                .times(tables.sin_inc_per_hz)
                .to_f32()
        });
        let mut phase = [0.0f64; 3];
        let mut decay = [0.0f64; 3];
        for (i, out) in self.grain.iter_mut().enumerate() {
            let mut g = 0.0f32;
            for k in 0..3 {
                let s = f64::from(tables.sin[ftol(phase[k]) as usize])
                    * f64::from(tables.exp[ftol(decay[k]) as usize]);
                // Store the first component instead of adding it to 0.0, like the DLL; the two
                // only differ in the sign of a zero, which no product here produces.
                g = if k == 0 {
                    s as f32
                } else {
                    (s + f64::from(g)) as f32
                };
                decay[k] += f64::from(tables.bw_step[k]);
                phase[k] += f64::from(inc[k]);
                if phase[k] >= SIN_LEN as f64 {
                    phase[k] -= SIN_LEN as f64;
                }
            }
            g = (f64::from(tables.breath[i]) * 0.5 + f64::from(g)) as f32;
            *out = tables.win[i] * g;
        }
    }

    /// Overlap-add the grain into the ring `offset` samples after the previous grain start.
    pub(super) fn add(&mut self, offset: usize) {
        self.write_pos = (self.write_pos + offset) % ACC_LEN;
        for (j, &g) in self.grain.iter().enumerate() {
            self.acc[(self.write_pos + j) % ACC_LEN] += g;
        }
    }

    /// Take the next sample out of the ring and clear its slot.
    pub(super) fn pop(&mut self) -> f32 {
        let sample = std::mem::take(&mut self.acc[self.read_pos]);
        self.read_pos = (self.read_pos + 1) % ACC_LEN;
        sample
    }
}
