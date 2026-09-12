//! Animate the mouth-opening value the editor reads.

use super::tables::Tables;
use super::x87::ftol;

/// Mouth-opening animation sequence for idle time (no key held); drives GUI param 6 only.
const MOUTH_SEQ: [f32; 24] = [
    0.16667, 0.1, 0.1333, 0.1, 0.0667, 0.0333, 0.0, 0.0333, //
    0.16667, 0.1, 0.1333, 0.1, 0.16667, 0.0333, 0.0, 0.0333, //
    0.0667, 0.1, 0.1333, 0.1, 0.16667, 0.0333, 0.0, 0.0333,
];

/// The mouth-opening value the GUI reads (param 6) and the idle animation that plays on it.
pub(super) struct MouthAnimation {
    /// mouth opening 0..1
    pub(super) value: f32,
    /// recompute the value from the vowel on the next gated sample
    needs_init: bool,
    /// position in `MOUTH_SEQ`
    index: usize,
    /// the idle timer and the sequence timer
    timer: i32,
    timer2: i32,
    /// the animation step and its multiples ×7, ×8.5, ×15, ×17, ×23
    step: i32,
    t7: i32,
    t85: i32,
    t15: i32,
    t17: i32,
    t23: i32,
}

impl MouthAnimation {
    pub(super) const fn new() -> Self {
        Self {
            value: 0.1667,
            needs_init: true,
            index: 0,
            timer: 0,
            timer2: 0,
            step: 0,
            t7: 0,
            t85: 0,
            t15: 0,
            t17: 0,
            t23: 0,
        }
    }

    /// Reset the animation state; keep the current value, like the DLL's reset.
    pub(super) fn reset(&mut self, tables: &Tables) {
        self.needs_init = true;
        self.index = 0;
        self.timer = 0;
        self.timer2 = 0;
        self.step = tables.mouth_step;
        self.t7 = self.step * 7;
        self.t85 = ftol(f64::from(self.step) * 8.5);
        self.t15 = self.step * 15;
        self.t17 = self.step * 17;
        self.t23 = self.step * 23;
    }

    /// Close the mouth when the voice stops. The DLL uses two slightly different constants.
    pub(super) fn release(&mut self, value: f32) {
        self.value = value;
        self.index = 0;
        self.needs_init = true;
    }

    /// Open the mouth for the vowel. Keep the DLL's `vowel · 24 · (1/30) + 0.2` rather than the
    /// equal `vowel · 0.8 + 0.2`: it counts animation frames (frame 6 of 30 for "u", 24 frames
    /// of opening for "i"), and the same operations keep the editor's frame choice identical.
    /// Only the editor reads the value, so f32 arithmetic is fine here.
    pub(super) fn sing(&mut self, vowel: f32) {
        self.value = vowel * 24.0 * 0.033_333_335 + 0.2;
    }

    /// One sample with a key down: hold the idle animation and apply a pending vowel.
    pub(super) fn gated_tick(&mut self, vowel: f32) {
        self.timer = 0;
        if self.needs_init {
            self.sing(vowel);
            self.needs_init = false;
        }
    }

    /// One idle sample: play the scripted mouth movements.
    pub(super) fn idle_tick(&mut self) {
        if self.timer == self.t7 || self.timer == self.t15 {
            self.value = 0.06667;
        }
        if self.timer == self.t85 || self.timer == self.t17 {
            self.value = 0.16667;
        }
        if self.timer2 >= self.step && self.timer >= self.t23 {
            if self.index >= MOUTH_SEQ.len() {
                self.index = 0;
            }
            self.timer2 = 0;
            self.value = MOUTH_SEQ[self.index];
            self.timer = self.t23;
            self.index += 1;
        }
    }

    /// Count one sample, gated or not. Wrap like the DLL's `int`: `timer2` only resets when the
    /// idle sequence fires, so a key held for 2^31 samples counts past `i32::MAX`.
    pub(super) fn advance(&mut self) {
        self.timer = self.timer.wrapping_add(1);
        self.timer2 = self.timer2.wrapping_add(1);
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values on purpose
mod tests {
    use super::*;
    use crate::engine::tables::Tables;

    /// Run the idle animation and return the sample at which the value changes, with the value.
    fn idle_changes(mouth: &mut MouthAnimation, samples: usize) -> Vec<(usize, f32)> {
        let mut changes = Vec::new();
        let mut last = mouth.value;
        for i in 0..samples {
            mouth.idle_tick();
            if mouth.value != last {
                last = mouth.value;
                changes.push((i, last));
            }
            mouth.advance();
        }
        changes
    }

    /// At 44.1 kHz the step is 9172 samples: the mouth twitches at 7 and 8.5 steps, then at
    /// 15 and 17, and from 23 steps on it plays `MOUTH_SEQ` one entry per step.
    #[test]
    fn idle_animation_keeps_the_dll_cadence() {
        let tables = Tables::new(44100.0);
        let mut mouth = MouthAnimation::new();
        mouth.reset(&tables);
        assert_eq!(mouth.step, 9172);
        let changes = idle_changes(&mut mouth, 26 * 9172);
        assert_eq!(
            &changes[..4],
            &[
                (7 * 9172, 0.06667),
                (77962, 0.16667), // 8.5 steps
                (15 * 9172, 0.06667),
                (17 * 9172, 0.16667),
            ]
        );
        // the sequence starts at 23 steps with its first entry, the resting value, so the first
        // visible change is the second entry one step later; then one entry per step
        assert_eq!(MOUTH_SEQ[0], 0.16667);
        assert_eq!(changes[4], (24 * 9172, MOUTH_SEQ[1]));
        assert_eq!(changes[5], (25 * 9172, MOUTH_SEQ[2]));
        assert_eq!(changes.len(), 6);
    }

    /// A key down opens the mouth for the vowel and holds the idle timer; a release closes it.
    #[test]
    fn singing_sets_the_mouth_from_the_vowel() {
        let tables = Tables::new(44100.0);
        let mut mouth = MouthAnimation::new();
        mouth.reset(&tables);
        mouth.gated_tick(0.5);
        assert_eq!(mouth.value, 0.5 * 24.0 * 0.033_333_335 + 0.2);
        mouth.release(0.16667);
        assert_eq!(mouth.value, 0.16667);
        assert!(mouth.needs_init);
    }
}
