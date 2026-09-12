//! Synthesis core of Delay Lama (AudioNerdz, 2002). Keep it a sample-exact mirror of
//! `Delay Lama.dll`.
//!
//! Read the voice as a FOF synth (CHANT style, after Xavier Rodet at IRCAM) that overlap-adds
//! one grain per pitch period:
//!
//! * Rebuild the 20 ms grain whenever the vowel or the head size changes. Sum three formant
//!   components `sin(2π·Fk·t) · exp(-π·BWk·t)` (F1..F3 from vowel-interpolated tables,
//!   BW = 32.5 / 47.5 / 62.5 Hz) and two fixed high formants at 3800 Hz (BW 180) and 4950 Hz
//!   (BW 150) at half amplitude, then shape the sum with a window (1.8 ms half-cosine attack,
//!   half-cosine decay over the last 6 ms).
//! * Overlap-add the grain into a circular accumulation buffer once per pitch period
//!   (period = sr / f0, with f0 read from a 1/32-semitone pitch table at the glided note plus
//!   vibrato).
//! * Pass the output through a stereo feedback delay (309.6 ms left, 398.4 ms right, feedback
//!   0.5) and a note-dependent gain `(2 - note/72) · volume`.
//!
//! Keep the arithmetic a mirror of the DLL's x87 code (f32 storage, wider intermediates,
//! truncating float-to-int casts) so that the output matches the DLL sample for sample.

// Keep the `as` casts: truncating float-to-int casts and the int/float index arithmetic mirror
// the DLL's `_ftol` calls and x87 code, so the cast lints report the intended behaviour.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]

mod delay;
mod glide;
mod grains;
mod midi;
mod mouth;
mod notes;
mod params;
mod smoothers;
mod tables;
mod vibrato;
mod x87;

pub use grains::ACC_LEN;
pub use midi::{ControlChange, MidiEvent, MidiMessage};
pub use params::{PROGRAMS, Param, Params, Program};

use delay::StereoDelay;
use glide::Glide;
use grains::Grains;
use midi::BendSpread;
use mouth::MouthAnimation;
use notes::NoteStack;
use smoothers::{BendSmoother, PitchSmoother};
use std::iter::Peekable;
use std::slice::Iter;
use tables::{FORMANT_LEN, PITCH_LEN, Tables};
use vibrato::Vibrato;
use x87::ftol;

/// The note and velocity the DLL's GUI pad sent to the host while pressed.
const PAD_NOTE: u8 = 40;
const PAD_VELOCITY: u8 = 64;
/// Messages `midi_out` holds between two `drain_midi_out` calls. The plugin emits at most three
/// per block; other callers lose messages beyond the limit rather than allocate.
const MIDI_OUT_CAPACITY: usize = 64;

/// The events of one block, consumed in order of `delta_frames`.
type Events<'a> = Peekable<Iter<'a, MidiEvent>>;

/// Whether the grain in the ring matches the current vowel and head size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GrainState {
    /// Up to date.
    Fresh,
    /// A parameter changed while a key was down: rebuild before the next trigger. The DLL
    /// rebuilds synchronously in `setParameter` instead; the output is the same.
    Stale,
    /// The voice sat idle: rebuild and trigger a grain at once when a key comes.
    Idle,
}

pub struct DelayLama {
    params: Params,
    /// the sample rate `reset()` applies
    sample_rate: f32,
    tables: Tables,
    grains: Grains,
    notes: NoteStack,
    glide: Glide,
    vibrato: Vibrato,
    bend: BendSmoother,
    expression: PitchSmoother,
    mouth: MouthAnimation,
    spread: BendSpread,
    delay: StereoDelay,

    /// a key (MIDI or GUI) is down
    gate: bool,
    /// glided note plus vibrato
    pitch: f32,
    /// current f0 in Hz
    freq: f32,
    /// current pitch period in samples
    period: usize,
    /// samples since the last grain trigger
    since_last_grain: usize,
    grain_state: GrainState,
    /// last vowel passed to `set_vowel`
    vowel_prev: f32,
    /// smoothing tick timer
    smooth_timer: i32,
    /// CC1 / 127
    mod_wheel: f32,
    /// CC7 · 0.001 (default 0.1)
    volume: f32,
    /// read-only GUI param 5, the smoothed expression 0..1
    expression_value: f32,
    /// Apply the DLL's delta-0 bend spreading; see `set_spread_pitch_bends`.
    spread_pitch_bends: bool,
    /// MIDI messages the original would send back to the host from GUI actions.
    midi_out: Vec<MidiMessage>,
}

impl DelayLama {
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let tables = Tables::new(sample_rate);
        let grains = Grains::new(tables.grain_len);
        let mut voice = DelayLama {
            params: Params::default(),
            sample_rate,
            tables,
            grains,
            notes: NoteStack::new(),
            glide: Glide::new(),
            vibrato: Vibrato::new(),
            bend: BendSmoother::new(),
            expression: PitchSmoother::new(),
            mouth: MouthAnimation::new(),
            spread: BendSpread::new(),
            delay: StereoDelay::new(),
            gate: false,
            pitch: 36.0,
            freq: 0.0,
            period: 0,
            since_last_grain: 0,
            grain_state: GrainState::Idle,
            vowel_prev: 0.5,
            smooth_timer: 0,
            mod_wheel: 0.0,
            volume: 0.1,
            expression_value: 0.0,
            spread_pitch_bends: true,
            midi_out: Vec::with_capacity(MIDI_OUT_CAPACITY),
        };
        voice.reset();
        voice
    }

    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        self.reset();
    }

    /// Store a new sample rate without rebuilding (the original applies it at the next
    /// `resume()`); call `reset()` afterwards.
    pub fn set_sample_rate_deferred(&mut self, sr: f32) {
        self.sample_rate = sr;
    }

    /// The longest block `process` accepts: the accumulation ring minus one grain.
    #[must_use]
    pub const fn max_block_len(&self) -> usize {
        ACC_LEN.saturating_sub(self.tables.grain_len)
    }

    /// Reset the voice, which the DLL does on `resume()`. Rebuild
    /// the tables only when the sample rate changed, like the DLL; keep the parameters, the
    /// glide position and the mouth value, which the DLL leaves alone as well. Only the rebuild
    /// allocates, and only the two sample-rate setters (which `activate()` and the script's
    /// `setsr` call outside the audio thread) change the rate, so a plain `reset()` from the
    /// audio thread stays allocation-free.
    #[allow(clippy::float_cmp)] // exact comparison on purpose: rebuild only on a real change
    pub fn reset(&mut self) {
        if self.tables.sample_rate != self.sample_rate {
            self.tables = Tables::new(self.sample_rate);
            self.grains = Grains::new(self.tables.grain_len);
        }
        self.grains.reset();
        self.notes.clear();
        self.delay.reset(&self.tables);
        self.bend.reset();
        self.expression.reset();
        self.gate = false;
        self.volume = 0.1;
        self.grains.build(&self.tables, 0.5, self.params.head_size);
        self.grain_state = GrainState::Idle;
        self.vowel_prev = 0.5;
        self.glide.active = false;
        self.pitch = 36.0;
        self.mod_wheel = 0.0;
        self.vibrato.reset(&self.tables);
        self.mouth.reset(&self.tables);
        self.since_last_grain = 0;
        self.smooth_timer = 0;
    }

    // ------------------------------------------------------------------ parameters

    /// The four user parameters.
    #[must_use]
    pub const fn params(&self) -> &Params {
        &self.params
    }

    /// Ask for a grain rebuild before the next trigger; an idle voice rebuilds anyway.
    fn mark_grain_stale(&mut self) {
        if self.grain_state == GrainState::Fresh {
            self.grain_state = GrainState::Stale;
        }
    }

    /// Glide time (param 0).
    pub fn set_port_time(&mut self, value: f32) {
        self.params.port_time = value;
    }

    /// The vowel (param 1); the pitch-bend smoother feeds it as well.
    #[allow(clippy::float_cmp)] // the DLL compares the vowel exactly
    pub fn set_vowel(&mut self, value: f32) {
        self.params.vowel = value;
        if self.gate {
            self.mouth.sing(value);
            if self.params.vowel != self.vowel_prev {
                self.mark_grain_stale(); // DLL: build_grain() right here
            }
        }
        self.vowel_prev = self.params.vowel;
    }

    /// Delay level (param 2).
    pub fn set_delay(&mut self, value: f32) {
        self.params.delay = value;
    }

    /// Head size (param 3).
    pub fn set_head_size(&mut self, value: f32) {
        self.params.head_size = value;
        if self.gate {
            self.mark_grain_stale(); // DLL: build_grain() right here
        }
    }

    /// Press the GUI pad (param 9 = 1): open the gate and echo the DLL's note on.
    pub fn pad_press(&mut self) {
        self.gate = true;
        self.emit(MidiMessage::NoteOn {
            note: PAD_NOTE,
            velocity: PAD_VELOCITY,
        });
    }

    /// Release the GUI pad (param 9 = 0): close the gate unless a key holds it, and echo the
    /// DLL's note off.
    pub fn pad_release(&mut self) {
        if self.notes.top() == 0 {
            self.gate = false;
            self.mouth.release(0.1667);
            self.emit(MidiMessage::NoteOff { note: PAD_NOTE });
        }
    }

    /// The pad's Y axis (param 10): the vowel, through the pitch-bend smoother.
    pub fn set_pad_vowel(&mut self, value: f32) {
        self.bend.pending_param = Some(value);
        let msb = ftol(f64::from(value) * 127.0) as u8;
        self.emit(MidiMessage::PitchBend { lsb: 0, msb });
    }

    /// The pad's X axis (param 11): the pitch, through the expression smoother.
    pub fn set_pad_pitch(&mut self, value: f32) {
        self.expression.pending = Some(value);
        let v = ftol(f64::from(value) * 127.0) as u8;
        self.emit(MidiMessage::ControlChange {
            controller: ControlChange::Expression.number(),
            value: v,
        });
    }

    /// The mouth opening 0..1 the editor animates from (param 6).
    #[must_use]
    pub const fn mouth(&self) -> f32 {
        self.mouth.value
    }

    /// The smoothed expression 0..1 the editor's pitch indicator follows (param 5).
    #[must_use]
    pub const fn expression(&self) -> f32 {
        self.expression_value
    }

    /// Mirror the DLL's VST `setParameter`, numbered like the DLL; the typed methods above do
    /// the work. Params 5 and 6 are the editor's read-backs, which the DLL's editor also wrote.
    pub fn set_parameter(&mut self, param: Param, value: f32) {
        match param {
            Param::PortTime => self.set_port_time(value),
            Param::Vowel => self.set_vowel(value),
            Param::Delay => self.set_delay(value),
            Param::HeadSize => self.set_head_size(value),
            Param::Expression => self.expression_value = value,
            Param::Mouth => {
                if !self.gate {
                    self.mouth.value = value;
                }
            }
            Param::PadGate => {
                if value == 0.0 {
                    self.pad_release();
                } else {
                    self.pad_press();
                }
            }
            Param::PadVowel => self.set_pad_vowel(value),
            Param::PadPitch => self.set_pad_pitch(value),
        }
    }

    /// Mirror the DLL's VST `getParameter`; the DLL reports 0 for the pad parameters.
    #[must_use]
    pub const fn get_parameter(&self, param: Param) -> f32 {
        match param {
            Param::PortTime => self.params.port_time,
            Param::Vowel => self.params.vowel,
            Param::Delay => self.params.delay,
            Param::HeadSize => self.params.head_size,
            Param::Expression => self.expression(),
            Param::Mouth => self.mouth(),
            Param::PadGate | Param::PadVowel | Param::PadPitch => 0.0,
        }
    }

    /// Load a factory program; clamp the index to the last one like the DLL's `setProgram`.
    pub fn set_program(&mut self, index: usize) {
        let program = PROGRAMS[index.min(PROGRAMS.len() - 1)];
        self.set_port_time(program.port_time);
        self.set_delay(program.delay);
        self.set_head_size(program.head_size);
    }

    /// Apply the DLL's delta-0 bend spreading (on by default, see `BendSpread`); turn it off for
    /// hosts that stamp events with sample offsets.
    pub fn set_spread_pitch_bends(&mut self, on: bool) {
        self.spread_pitch_bends = on;
    }

    // ------------------------------------------------------------------ MIDI

    /// Note on; velocity 0 acts as a note off, like MIDI and the DLL.
    pub fn note_on(&mut self, note: u8, velocity: u8) {
        self.note_on_off(note, velocity);
    }

    pub fn note_off(&mut self, note: u8) {
        self.note_on_off(note, 0);
    }

    /// Release every held key, top first, through the note-off path; the DLL had no such
    /// message, and hosts send one as a wildcard note off.
    pub fn all_notes_off(&mut self) {
        while self.notes.top() != 0 {
            self.note_off(self.notes.top() + 12);
        }
    }

    /// The DLL's note handler. Accept only MIDI notes 16..=84. Keep the synth monophonic with
    /// a last-note-priority stack, where a second held key enables portamento (legato).
    fn note_on_off(&mut self, midi_note: u8, velocity: u8) {
        let n = i32::from(midi_note) - 12;
        if (4..=72).contains(&n) {
            if velocity == 0 {
                self.notes.remove(n as u8);
            } else {
                self.notes.push(n as u8);
            }
        }
        let cur = self.notes.top();
        self.glide.target = f32::from(cur);
        self.gate = cur != 0;
        if cur == 0 {
            self.glide.active = false;
            self.mouth.release(0.16667);
        }
        if self.notes.has_second() && !self.glide.active {
            self.glide.active = true;
        }
    }

    /// Handle a control change.
    pub fn control_change(&mut self, cc: ControlChange, value: u8) {
        let v = f32::from(value) * 0.007_874_016; // /127
        match cc {
            ControlChange::ModWheel => self.mod_wheel = v,
            ControlChange::Portamento => self.set_port_time(v),
            ControlChange::Volume => self.volume = f32::from(value) * 0.001,
            ControlChange::Expression => self.expression.pending = Some(v),
            ControlChange::DelayLevel => self.set_delay(v),
            ControlChange::HeadSize => self.set_head_size(v),
        }
    }

    /// Handle a 14-bit pitch bend (LSB, MSB); it drives the vowel, not the pitch.
    pub fn pitch_bend(&mut self, lsb: u8, msb: u8) {
        self.bend.pending_midi = Some((lsb & 0x7f, msb & 0x7f));
    }

    /// The MIDI messages the original sent to the host for the GUI pad (note 40, CC11 and
    /// pitch bend) since the last call. Drain them once per block; the buffer holds
    /// `MIDI_OUT_CAPACITY` messages and drops the rest rather than grow.
    pub fn drain_midi_out(&mut self) -> impl Iterator<Item = MidiMessage> + '_ {
        self.midi_out.drain(..)
    }

    /// Queue a message for `drain_midi_out` without allocating.
    fn emit(&mut self, message: MidiMessage) {
        if self.midi_out.len() < self.midi_out.capacity() {
            self.midi_out.push(message);
        }
    }

    /// Consume the MIDI events due at sample `i` of a block of `n` samples, like the DLL's
    /// per-sample event handler.
    fn handle_midi(&mut self, events: &mut Events<'_>, i: i32, n: i32) {
        let mut collected = 0i32;
        while let Some(event) = events.next_if(|event| event.delta_frames <= i) {
            match event.message {
                MidiMessage::NoteOn { note, velocity } => self.note_on(note, velocity),
                MidiMessage::NoteOff { note } => self.note_off(note),
                MidiMessage::ControlChange { controller, value } => {
                    // the DLL ignores every other controller
                    if let Some(cc) = ControlChange::from_number(controller) {
                        self.control_change(cc, value);
                    }
                }
                MidiMessage::PitchBend { lsb, msb } => {
                    if i == 0 && self.spread_pitch_bends {
                        if self.spread.collect(collected == 0, (lsb, msb)) {
                            collected += 1;
                        }
                    } else {
                        self.pitch_bend(lsb, msb);
                    }
                }
            }
        }
        if collected != 0 {
            self.spread.schedule(collected, n);
        }
        if let Some((lsb, msb)) = self.spread.due(i) {
            self.pitch_bend(lsb, msb);
        }
    }

    // ------------------------------------------------------------------ DSP

    /// The control-rate work for one sample of the block: events, smoothing, glide, vibrato,
    /// grain triggering, mouth animation.
    fn control_tick(&mut self, events: &mut Events<'_>, i: i32, n: i32) {
        self.handle_midi(events, i, n);

        let steps = self.tables.smooth_steps;
        self.bend.apply_pending(steps);
        self.expression.apply_pending(steps);
        if self.smooth_timer >= self.tables.smooth_interval {
            self.smooth_timer = 0;
            if let Some(bend) = self.bend.step() {
                let vowel = bend as f32 * 6.103_888e-05; // ≈ 1/16383
                self.set_vowel(vowel);
            }
            if let Some((note, read_back)) = self.expression.step() {
                self.expression_value = read_back;
                self.glide.target = note;
            }
        }

        if self.gate {
            self.mouth.gated_tick(self.params.vowel);
            self.pitch = self.glide.tick(self.params.port_time, self.sample_rate);
            let vibrato = self.vibrato.tick(self.mod_wheel, &self.tables);
            // Mirror the DLL's FST: store the rounded pitch, but compute the table index below
            // from the unrounded extended-precision sum that stays on the FPU stack.
            let pitch_ext = f64::from(vibrato) + f64::from(self.pitch); // exact
            self.pitch = pitch_ext as f32;

            // f0 from the 1/32-semitone pitch table, period in samples. Multiply by -32 and
            // negate like the DLL; truncation is symmetric, so this equals `ftol(pitch_ext * 32)`.
            // The clamp is a safety net the DLL lacks: notes 4..=72 plus vibrato stay inside.
            let index = ftol(pitch_ext * f64::from(-32.0f32)).saturating_neg();
            self.freq = self.tables.pitch[index.clamp(0, PITCH_LEN as i32 - 1) as usize];
            self.period = ftol(f64::from(self.sample_rate) / f64::from(self.freq)).max(0) as usize;
            if self.since_last_grain >= self.period || self.grain_state == GrainState::Idle {
                if self.grain_state != GrainState::Fresh {
                    self.grains
                        .build(&self.tables, self.params.vowel, self.params.head_size);
                }
                self.grains.add(self.since_last_grain);
                self.since_last_grain = 0;
                self.grain_state = GrainState::Fresh;
            }
        } else {
            // idle: emit no grains (no release stage); only animate the GUI mouth
            self.grain_state = GrainState::Idle;
            self.mouth.idle_tick();
        }

        self.vibrato.advance();
        self.mouth.advance();
        self.since_last_grain += 1;
        self.smooth_timer = self.smooth_timer.wrapping_add(1);
    }

    /// Render one block (the DLL's `processReplacing`). Pass `events` sorted by `delta_frames`;
    /// an event out of order applies at the sample where the walk reaches it, like the DLL's
    /// queue cursor. Keep the two passes: the output pass reads the block's final glide state
    /// on purpose.
    pub fn process(&mut self, events: &[MidiEvent], left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let mut events = events.iter().peekable();

        if self.since_last_grain >= ACC_LEN {
            self.since_last_grain -= ACC_LEN;
        }

        for i in 0..n {
            self.control_tick(&mut events, i as i32, n as i32);
        }

        // ring read, stereo feedback delay, note-dependent gain
        let gain = (f64::from(self.glide.note) * f64::from(-0.013_888_889f32) + 2.0)
            * f64::from(self.volume);
        for (out_l, out_r) in left.iter_mut().zip(right.iter_mut()) {
            let dry = self.grains.pop();
            let (wet_l, wet_r) = self.delay.tick(dry, self.params.delay);
            *out_l = ((f64::from(dry) + f64::from(wet_l)) * gain) as f32;
            *out_r = ((f64::from(dry) + f64::from(wet_r)) * gain) as f32;
        }
    }

    // ------------------------------------------------------------------ introspection
    #[must_use]
    pub const fn current_frequency(&self) -> f32 {
        self.freq
    }
    #[must_use]
    pub const fn is_gated(&self) -> bool {
        self.gate
    }
    #[must_use]
    pub fn grain(&self) -> &[f32] {
        &self.grains.grain
    }
    #[must_use]
    pub fn formant_freqs(&self, vowel: f32) -> [f32; 3] {
        let index = ftol(f64::from(vowel) * 1279.0).clamp(0, FORMANT_LEN as i32 - 1) as usize;
        std::array::from_fn(|k| self.tables.formants[k][index])
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values on purpose
mod tests {
    use super::*;

    fn render(voice: &mut DelayLama, frames: usize) {
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        voice.process(&[], &mut left, &mut right);
    }

    #[test]
    fn parameter_facade_reads_back_what_it_wrote() {
        let mut voice = DelayLama::new(48000.0);
        assert_eq!(voice.sample_rate(), 48000.0);
        for param in [Param::PortTime, Param::Vowel, Param::Delay, Param::HeadSize] {
            voice.set_parameter(param, 0.3);
            assert_eq!(voice.get_parameter(param), 0.3, "{param:?}");
        }
        voice.set_parameter(Param::Expression, 0.7);
        assert_eq!(voice.expression(), 0.7);
        voice.set_parameter(Param::Mouth, 0.4);
        assert_eq!(voice.mouth(), 0.4);
        assert_eq!(voice.get_parameter(Param::PadGate), 0.0);
        assert_eq!(voice.params().delay, 0.3);
    }

    /// The formant tables pass near the five vowels' knots (the index truncates, so the
    /// middle vowels land one entry before their knot).
    #[test]
    fn formant_freqs_follow_the_vowel_knots() {
        let voice = DelayLama::new(44100.0);
        assert_eq!(voice.formant_freqs(0.0), [280.0, 600.0, 2240.0]);
        let near =
            |got: [f32; 3], want: [f32; 3]| got.iter().zip(want).all(|(g, w)| (g - w).abs() < 2.0);
        assert!(near(voice.formant_freqs(0.5), [800.0, 1150.0, 2900.0]));
        assert!(near(voice.formant_freqs(1.0), [270.0, 2140.0, 2950.0]));
    }

    /// A4 (MIDI 69) sings an octave down, near 220 Hz plus vibrato.
    #[test]
    fn a_held_key_gates_the_voice_near_its_pitch() {
        let mut voice = DelayLama::new(44100.0);
        assert!(!voice.is_gated());
        voice.note_on(69, 100);
        render(&mut voice, 4410);
        assert!(voice.is_gated());
        let hz = voice.current_frequency();
        assert!((205.0..235.0).contains(&hz), "{hz}");
        assert_eq!(voice.grain().len(), 882);
        voice.note_off(69);
        assert!(!voice.is_gated());
    }

    #[test]
    fn all_notes_off_releases_every_held_key() {
        let mut voice = DelayLama::new(44100.0);
        for note in [60, 64, 67, 64] {
            voice.note_on(note, 100);
        }
        assert!(voice.is_gated());
        voice.all_notes_off();
        assert!(!voice.is_gated());
        assert_eq!(voice.notes.top(), 0);
    }
}
