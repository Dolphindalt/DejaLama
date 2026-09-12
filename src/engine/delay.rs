//! Run the stereo feedback delay.

use super::tables::Tables;

/// Stereo delay line length in samples; fixed, independent of the sample rate.
const DELAY_LEN: usize = 20000;

/// Two independent feedback combs (309.6 ms left, 398.4 ms right, feedback 0.5) in fixed
/// 20000-sample lines, which the read offsets wrap above 50 kHz like the DLL.
pub(super) struct StereoDelay {
    /// the two delay lines
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// write index
    write: usize,
    /// read offsets from the write index: the read indices advance in
    /// lockstep with it, so their distance stays what the initial values set
    read_offset_l: usize,
    read_offset_r: usize,
    /// feedback, 0.5
    feedback: f32,
}

impl StereoDelay {
    pub(super) fn new() -> Self {
        Self {
            left: vec![0.0; DELAY_LEN].into_boxed_slice(),
            right: vec![0.0; DELAY_LEN].into_boxed_slice(),
            write: 0,
            read_offset_l: 0,
            read_offset_r: 0,
            feedback: 0.5,
        }
    }

    pub(super) fn reset(&mut self, tables: &Tables) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.write = 0;
        let len = DELAY_LEN as i32;
        self.read_offset_l = tables.delay_read_l.rem_euclid(len) as usize;
        self.read_offset_r = tables.delay_read_r.rem_euclid(len) as usize;
        self.feedback = 0.5;
    }

    /// Feed one dry sample at level `mix`; return the wet left and right samples.
    pub(super) fn tick(&mut self, dry: f32, mix: f32) -> (f32, f32) {
        let w = self.write;
        let rl = (w + self.read_offset_l) % DELAY_LEN;
        let rr = (w + self.read_offset_r) % DELAY_LEN;
        let (fb, mix, dry) = (f64::from(self.feedback), f64::from(mix), f64::from(dry));
        self.left[w] = ((f64::from(self.left[rl]) * fb + dry) * mix) as f32;
        self.right[w] = ((f64::from(self.right[rr]) * fb + dry) * mix) as f32;
        self.write = (w + 1) % DELAY_LEN;
        // read after the write, like the DLL
        (self.left[rl], self.right[rr])
    }
}
