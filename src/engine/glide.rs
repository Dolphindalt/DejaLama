//! Glide the played note toward its target in legato playing.

/// Portamento: a constant rate of 12 semitones per (`port_time` + 0.01) seconds, snapping within
/// 0.2 semitone, only in legato playing.
pub(super) struct Glide {
    /// note the glide heads to (MIDI note - 12, or 36..48 from expression)
    pub(super) target: f32,
    /// current (glided) note; also drives the output gain
    pub(super) note: f32,
    /// semitones per sample while gliding
    inc: f32,
    /// legato: on from the second held key until all keys go up
    pub(super) active: bool,
}

impl Glide {
    pub(super) const fn new() -> Self {
        Self {
            target: 0.0,
            note: 0.0,
            inc: 0.0,
            active: false,
        }
    }

    /// Advance one sample and return the note to play.
    pub(super) fn tick(&mut self, port_time: f32, sample_rate: f32) -> f32 {
        if self.active {
            let (t, g) = (f64::from(self.target), f64::from(self.note));
            let seconds = (f64::from(port_time) + f64::from(0.01f32)) * f64::from(sample_rate);
            if t + f64::from(0.2f32) < g {
                self.inc = (-12.0f64 / seconds) as f32;
            } else if t - f64::from(0.2f32) > g {
                self.inc = (12.0f64 / seconds) as f32;
            } else {
                self.note = self.target;
                self.inc = 0.0;
            }
            self.note += self.inc;
        } else {
            self.note = self.target;
        }
        self.note
    }
}
