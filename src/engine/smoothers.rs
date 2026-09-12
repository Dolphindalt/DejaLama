//! Ramp pitch-bend and expression values to their targets in 10 steps of 10 ms.

use super::x87::ftol;

/// The centre of the 14-bit pitch-bend range, the smoother's starting value.
const BEND_CENTRE: i32 = 0x2000;

/// The 14-bit pitch-bend value ramped to its target in 10 steps of 10 ms, in integer arithmetic
/// like the DLL; it drives the vowel.
pub(super) struct BendSmoother {
    /// current 14-bit bend value (starts centred, 8192)
    current: i32,
    /// bend units per step (integer division)
    step: i32,
    /// steps still to take
    steps_left: i32,
    /// a bend from MIDI waiting for the next control tick
    pub(super) pending_midi: Option<(u8, u8)>,
    /// a bend from the GUI pad (0..1) waiting for the next control tick
    pub(super) pending_param: Option<f32>,
}

impl BendSmoother {
    pub(super) const fn new() -> Self {
        Self {
            current: BEND_CENTRE,
            step: 0,
            steps_left: 0,
            pending_midi: None,
            pending_param: None,
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    fn retarget(&mut self, target: i32, steps: i32) {
        self.step = (target - self.current) / steps;
        self.steps_left = steps;
    }

    /// Turn pending bends into a ramp: MIDI first, then the pad, like the DLL's flag order.
    pub(super) fn apply_pending(&mut self, steps: i32) {
        if let Some((lsb, msb)) = self.pending_midi.take() {
            self.retarget(i32::from(msb) * 0x80 + i32::from(lsb), steps);
        }
        if let Some(value) = self.pending_param.take() {
            self.retarget(ftol(f64::from(value) * 16384.0), steps);
        }
    }

    /// Take one ramp step and return the new bend value, if a ramp runs.
    pub(super) fn step(&mut self) -> Option<i32> {
        if self.steps_left > 0 {
            self.steps_left -= 1;
            self.current += self.step;
            Some(self.current)
        } else {
            None
        }
    }
}

/// The expression value (CC11 or the pad's X axis) ramped to a note in 36..48 in 10 steps of
/// 10 ms, in float arithmetic like the DLL; it drives the pitch.
pub(super) struct PitchSmoother {
    /// current smoothed note (starts at 36)
    current: f32,
    /// semitones per step
    step: f32,
    /// steps still to take
    steps_left: i32,
    /// an expression value (0..1) waiting for the next control tick
    pub(super) pending: Option<f32>,
}

impl PitchSmoother {
    pub(super) const fn new() -> Self {
        Self {
            current: 36.0,
            step: 0.0,
            steps_left: 0,
            pending: None,
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    /// Turn a pending expression value into a ramp. Mirror the DLL's FST, which keeps the
    /// unrounded target on the FPU stack for the difference.
    pub(super) fn apply_pending(&mut self, steps: i32) {
        if let Some(value) = self.pending.take() {
            let target = f64::from(value) * 12.0 + 36.0; // exact
            let delta = target - f64::from(self.current); // exact
            self.step = (delta / f64::from(steps)) as f32;
            self.steps_left = steps;
        }
    }

    /// Take one ramp step and return the new note and its 0..1 read-back, if a ramp runs.
    pub(super) fn step(&mut self) -> Option<(f32, f32)> {
        if self.steps_left > 0 {
            self.steps_left -= 1;
            let cur = f64::from(self.step) + f64::from(self.current); // exact
            self.current = cur as f32;
            Some((
                self.current,
                ((cur - 36.0) * f64::from(0.083_333_336f32)) as f32,
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The integer ramp from the centre to 16383 covers 10 · ((16383 - 8192) / 10) = 8190 and
    /// stops at 16382.
    #[test]
    fn bend_ramp_uses_integer_steps() {
        let mut bend = BendSmoother::new();
        assert_eq!(bend.step(), None);
        bend.pending_midi = Some((0x7f, 0x7f));
        bend.apply_pending(10);
        let values: Vec<i32> = std::iter::from_fn(|| bend.step()).collect();
        assert_eq!(values.len(), 10);
        assert_eq!((values[0], values[9]), (8192 + 819, 16382));
    }

    /// A pad value and a MIDI bend pending at once: the pad wins, like the DLL's flag order.
    #[test]
    fn pad_bend_overrides_midi_bend() {
        let mut bend = BendSmoother::new();
        bend.pending_midi = Some((0, 0));
        bend.pending_param = Some(1.0);
        bend.apply_pending(10);
        assert_eq!(bend.step(), Some(8192 + (16384 - 8192) / 10));
    }

    #[test]
    fn expression_ramps_to_a_note_between_36_and_48() {
        let mut pitch = PitchSmoother::new();
        assert_eq!(pitch.step(), None);
        pitch.pending = Some(1.0);
        pitch.apply_pending(10);
        let steps: Vec<(f32, f32)> = std::iter::from_fn(|| pitch.step()).collect();
        assert_eq!(steps.len(), 10);
        assert!((steps[0].0 - 37.2).abs() < 1e-5);
        let (note, read_back) = steps[9];
        assert!((note - 48.0).abs() < 1e-4, "{note}");
        assert!((read_back - 1.0).abs() < 1e-5, "{read_back}");
    }
}
