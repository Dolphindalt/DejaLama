//! Deja Lama, a one-to-one re-creation of AudioNerdz' Delay Lama (2002) as a CLAP/VST3
//! instrument.
//!
//! Keep the synthesis in `engine` (bit-exact against the original DLL, see
//! `tests/engine_regression.rs`) and only the nice-plug glue in this file.

use nice_plug::prelude::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub mod engine;
pub mod gui;
// the embedded variant leaves the fetching half of the module to build.rs
#[cfg_attr(feature = "embed-assets", allow(dead_code))]
mod original;
pub mod script;
pub mod shared;

use engine::{DelayLama, MidiEvent, MidiMessage};
use shared::GuiShared;

pub struct DejaLama {
    params: Arc<DejaLamaParams>,
    engine: DelayLama,
    shared: Arc<GuiShared>,
    /// Last pad gate value forwarded to the engine (param 9).
    pad_gate_pushed: Option<bool>,
    /// Last pad X and Y forwarded to the engine (params 11 and 10).
    pad_pushed: Option<(f32, f32)>,
    /// Parameter values last pushed into the engine, `None` until the first push. Forward only
    /// changes: CC5, CC12, CC13 and pitch bend also move the engine's values, and a push every
    /// block would undo them.
    pushed: [Option<f32>; 4],
    /// Per-block MIDI event scratch buffer. Keep it pre-allocated: never allocate on the audio
    /// thread.
    events: Vec<MidiEvent>,
}

#[derive(Params)]
pub struct DejaLamaParams {
    /// Portamento: 12 semitones per (value + 0.01) seconds, in legato playing only.
    #[id = "porttime"]
    pub port_time: FloatParam,
    /// 0 = u, 0.25 = o, 0.5 = a, 0.75 = e, 1 = i. MIDI pitch bend moves this too.
    #[id = "vowel"]
    pub vowel: FloatParam,
    /// Level of the stereo feedback delay (309.6 ms / 398.4 ms).
    #[id = "delay"]
    pub delay: FloatParam,
    /// Formant scale 0.75 + 0.5·value (baritone to soprano).
    #[id = "headsize"]
    pub head_size: FloatParam,

    // The XY pad of the original GUI. The original sent these to the host as MIDI (note 40,
    // CC11, pitch bend) so that a DAW could record the moves; expose them as parameters instead
    // so that automation records them.
    /// Pad pressed (original param 9): the monk sings while this is on.
    #[id = "padgate"]
    pub pad_gate: BoolParam,
    /// Pad X (original param 11): pitch, MIDI note 36 + 12·x; the engine smooths it.
    #[id = "padx"]
    pub pad_x: FloatParam,
    /// Pad Y (original param 10): vowel; the engine smooths it like a pitch bend.
    #[id = "pady"]
    pub pad_y: FloatParam,
}

impl Default for DejaLamaParams {
    fn default() -> Self {
        // Use program 0, "Rabten", as the defaults (see engine::PROGRAMS for the other four).
        let unit = FloatRange::Linear { min: 0.0, max: 1.0 };
        Self {
            port_time: FloatParam::new("PortTime", 0.5, unit)
                // the original displays this as "<value·1000> Hours"; keep the joke
                .with_unit(" Hours")
                .with_value_to_string(Arc::new(|v| format!("{:.0}", v * 1000.0)))
                .with_string_to_value(Arc::new(|s| number(s, "Hours").map(|hours| hours / 1000.0))),
            vowel: FloatParam::new("Vowel", 0.5, unit)
                .with_value_to_string(Arc::new(|v| format!("{v:.2} ({})", vowel_name(v))))
                .with_string_to_value(Arc::new(parse_vowel)),
            delay: FloatParam::new("Delay", 0.8, unit)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
                .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            head_size: FloatParam::new("HeadSize", 0.5, unit)
                .with_unit(" cm")
                .with_value_to_string(Arc::new(|v| format!("{:.1}", v * 30.0)))
                .with_string_to_value(Arc::new(|s| number(s, "cm").map(|cm| cm / 30.0))),
            pad_gate: BoolParam::new("Pad Gate", false).hide_in_generic_ui(),
            pad_x: FloatParam::new("Pad X (pitch)", 0.0, unit).hide_in_generic_ui(),
            pad_y: FloatParam::new("Pad Y (vowel)", 0.5, unit).hide_in_generic_ui(),
        }
    }
}

impl Default for DejaLama {
    fn default() -> Self {
        let mut engine = DelayLama::new(44100.0);
        // Skip the original's hack that spreads pitch bends arriving at offset 0 over the block:
        // VST3/CLAP events carry sample offsets.
        engine.set_spread_pitch_bends(false);
        Self {
            params: Arc::new(DejaLamaParams::default()),
            engine,
            shared: Arc::new(GuiShared::default()),
            pad_gate_pushed: None,
            pad_pushed: None,
            pushed: [None; 4],
            events: Vec::with_capacity(Self::INPUT_EVENT_CAPACITY),
        }
    }
}

/// Name the vowel nearest to a Vowel parameter value (u, o, a, e, i at 0, 0.25, 0.5, 0.75, 1).
fn vowel_name(v: f32) -> &'static str {
    match v {
        v if v < 0.125 => "u",
        v if v < 0.375 => "o",
        v if v < 0.625 => "a",
        v if v < 0.875 => "e",
        _ => "i",
    }
}

/// Parse a number the user typed, with or without the unit some hosts pass along.
fn number(text: &str, unit: &str) -> Option<f32> {
    text.trim().trim_end_matches(unit).trim().parse().ok()
}

/// Parse a Vowel entry: the number the display shows (`0.50 (a)`), or a vowel letter.
fn parse_vowel(text: &str) -> Option<f32> {
    let text = text.trim();
    let vowels = [("u", 0.0), ("o", 0.25), ("a", 0.5), ("e", 0.75), ("i", 1.0)];
    if let Some((_, value)) = vowels
        .iter()
        .find(|(vowel, _)| text.eq_ignore_ascii_case(vowel))
    {
        return Some(*value);
    }
    text.split_whitespace()
        .next()
        .and_then(|word| word.parse().ok())
}

/// Keep a host parameter inside the range the engine indexes tables with. Live automation
/// stays in 0..1, but a state file restores any value.
fn guard(value: f32) -> Option<f32> {
    value.is_finite().then(|| value.clamp(0.0, 1.0))
}

/// Whether to forward the pad position: always while the gate is on, on a change while it is
/// off (a tap shorter than a block), never for the resting values after a reset, which would
/// override the host's Vowel parameter.
fn pad_forward(previous: Option<(f32, f32)>, gate: bool, pad: (f32, f32)) -> bool {
    gate || previous.is_some_and(|last| last != pad)
}

/// Scale a normalised value to a 7-bit MIDI data byte.
fn midi_7bit(v: f32) -> u8 {
    // the clamp bounds the value, so the cast cannot truncate or lose the sign
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = (v * 127.0).round().clamp(0.0, 127.0) as u8;
    byte
}

/// Scale a normalised pitch-bend value to its 14-bit LSB and MSB bytes.
fn midi_14bit(v: f32) -> (u8, u8) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let raw = (v * 16383.0).round().clamp(0.0, 16383.0) as u16;
    // both halves fit in 7 bits after the mask and the shift
    #[allow(clippy::cast_possible_truncation)]
    let (lsb, msb) = ((raw & 0x7f) as u8, (raw >> 7) as u8);
    (lsb, msb)
}

/// Clamp a buffer offset to the engine's `i32` frame index.
fn delta(timing: u32) -> i32 {
    i32::try_from(timing).unwrap_or(i32::MAX)
}

impl DejaLama {
    /// The lowest sample rate the engine's tables make sense at; hosts offer 8 kHz and up.
    const MIN_SAMPLE_RATE: f32 = 1000.0;

    /// Forward the host parameters that changed since the last block.
    fn sync_params(&mut self) {
        let values = [
            self.params.port_time.value(),
            self.params.vowel.value(),
            self.params.delay.value(),
            self.params.head_size.value(),
        ];
        let setters: [fn(&mut DelayLama, f32); 4] = [
            DelayLama::set_port_time,
            DelayLama::set_vowel,
            DelayLama::set_delay,
            DelayLama::set_head_size,
        ];
        for ((pushed, v), set) in self.pushed.iter_mut().zip(values).zip(setters) {
            let Some(v) = guard(v) else { continue };
            if *pushed != Some(v) {
                *pushed = Some(v);
                set(&mut self.engine, v);
            }
        }
    }

    /// Forward the pad: the gate on a change, X and Y by the rule in `pad_forward`.
    fn sync_pad(&mut self) {
        let gate = self.params.pad_gate.value();
        if self.pad_gate_pushed != Some(gate) {
            self.pad_gate_pushed = Some(gate);
            if gate {
                self.engine.pad_press();
            } else {
                self.engine.pad_release();
            }
        }
        let (Some(x), Some(y)) = (
            guard(self.params.pad_x.value()),
            guard(self.params.pad_y.value()),
        ) else {
            return;
        };
        let forward = pad_forward(self.pad_pushed, gate, (x, y));
        self.pad_pushed = Some((x, y));
        if forward {
            self.engine.set_pad_pitch(x);
            self.engine.set_pad_vowel(y);
        }
    }
}

impl Plugin for DejaLama {
    const NAME: &'static str = "Deja Lama";
    const VENDOR: &'static str = "dcaron";
    const URL: &'static str = "https://github.com/dolphindalt/DejaLama";
    const EMAIL: &'static str = "dpcaron99@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Pure synth: no audio input, stereo output.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[],
        aux_output_ports: &[],
        names: PortNames::const_default(),
    }];

    // Notes, the CCs in `engine::ControlChange` and pitch bend (= vowel). On VST3 the wrapper
    // exposes the CCs as hidden parameters, like nih-plug.
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type Editor = nice_plug_egui::EguiEditor<gui::DejaLamaGui>;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        gui::create_editor(self.params.clone(), self.shared.clone())
    }

    fn activate(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        let sample_rate = buffer_config.sample_rate;
        if !(sample_rate.is_finite() && sample_rate >= Self::MIN_SAMPLE_RATE) {
            nice_error!(
                "Deja Lama needs a sample rate of at least {} Hz; the host offers {sample_rate}",
                Self::MIN_SAMPLE_RATE
            );
            return false;
        }
        // Allocate and build all tables here (the engine rebuilds them only when the rate
        // changed); the host calls `reset()` right after this.
        self.engine.set_sample_rate(sample_rate);
        // the original fixes the accumulation ring at 10240 samples
        let max_block = self.engine.max_block_len();
        let fits = buffer_config.max_buffer_size as usize <= max_block;
        if !fits {
            nice_error!(
                "Deja Lama needs blocks of at most {max_block} samples at {sample_rate} Hz; the host offers {}",
                buffer_config.max_buffer_size
            );
        }
        fits
    }

    fn reset(&mut self) {
        self.engine.reset(); // allocation-free after activate()
        self.pushed = [None; 4]; // re-push the parameters on the next block
        self.pad_gate_pushed = None;
        self.pad_pushed = None;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.sync_params();
        // The original sent the pad's note, CC11 and pitch bend to the host; the port exposes
        // the pad as parameters instead (see AGENTS.md), so drop the echo.
        self.engine.drain_midi_out().for_each(drop);
        // The original re-sent X and Y about every 4 ms while the pad was held, which restarted
        // the smoothing ramps each time; re-sending them every block while the gate is on
        // reproduces that.
        self.sync_pad();

        // Translate the host's events into the engine's messages.
        self.events.clear();
        while let Some(event) = context.next_event() {
            let (timing, message) = match event {
                NoteEvent::NoteOn {
                    timing,
                    key,
                    velocity,
                    ..
                } => match key.number() {
                    Some(note) => (
                        timing,
                        MidiMessage::NoteOn {
                            note,
                            velocity: midi_7bit(velocity),
                        },
                    ),
                    None => continue,
                },
                NoteEvent::NoteOff { timing, key, .. } | NoteEvent::Choke { timing, key, .. } => {
                    let Some(note) = key.number() else {
                        // a wildcard releases every key; the original had no such message, so
                        // apply it at the start of the block
                        self.engine.all_notes_off();
                        continue;
                    };
                    (timing, MidiMessage::NoteOff { note })
                }
                NoteEvent::MidiCC {
                    timing, cc, value, ..
                } => (
                    timing,
                    MidiMessage::ControlChange {
                        controller: cc,
                        value: midi_7bit(value),
                    },
                ),
                NoteEvent::MidiPitchBend { timing, value, .. } => {
                    let (lsb, msb) = midi_14bit(value);
                    (timing, MidiMessage::PitchBend { lsb, msb })
                }
                _ => continue,
            };
            if self.events.len() < self.events.capacity() {
                self.events.push(MidiEvent {
                    delta_frames: delta(timing),
                    message,
                });
            }
        }

        let [left, right] = buffer.as_slice() else {
            return ProcessStatus::Error("expected a stereo output");
        };
        self.engine.process(&self.events, left, right);

        // publish the animation state for the editor
        self.shared
            .mouth
            .store(self.engine.mouth(), Ordering::Relaxed);
        self.shared
            .vowel
            .store(self.engine.params().vowel, Ordering::Relaxed);
        self.shared
            .expression
            .store(self.engine.expression(), Ordering::Relaxed);

        // Never let the host put the plugin to sleep: the feedback delay rings for seconds, and
        // the idle mouth animation the editor shows runs from this loop.
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for DejaLama {
    const CLAP_ID: &'static str = "com.dolphindalt.deja-lama";
    const CLAP_DESCRIPTION: Option<&'static str> = Some(
        "Unofficial one-to-one re-creation of AudioNerdz' Delay Lama (2002), the virtual singing monk",
    );
    const CLAP_MANUAL_URL: Option<&'static str> = Some("https://github.com/dolphindalt/DejaLama");
    const CLAP_SUPPORT_URL: Option<&'static str> =
        Some("https://github.com/dolphindalt/DejaLama/issues");
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for DejaLama {
    const VST3_CLASS_ID: [u8; 16] = *b"DejaLama-dolphin";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(DejaLama);
nice_export_vst3!(DejaLama);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vowel_names_switch_halfway_between_the_vowels() {
        let names: Vec<&str> = [
            0.0, 0.124, 0.125, 0.374, 0.375, 0.624, 0.625, 0.874, 0.875, 1.0,
        ]
        .into_iter()
        .map(vowel_name)
        .collect();
        assert_eq!(names, ["u", "u", "o", "o", "a", "a", "e", "e", "i", "i"]);
    }

    #[test]
    fn midi_bytes_round_trip_through_nice_plug_scaling() {
        assert_eq!(
            (midi_7bit(0.0), midi_7bit(0.5), midi_7bit(1.0)),
            (0, 64, 127)
        );
        assert_eq!(midi_7bit(2.0), 127);
        assert_eq!(midi_14bit(0.0), (0, 0));
        assert_eq!(midi_14bit(8192.0 / 16383.0), (0, 64)); // centre
        assert_eq!(midi_14bit(1.0), (127, 127));
    }

    #[test]
    fn text_entry_reads_the_displayed_text_back() {
        assert_eq!(number("500 Hours", "Hours"), Some(500.0));
        assert_eq!(number(" 15.0 cm ", "cm"), Some(15.0));
        assert_eq!(number("15.0", "cm"), Some(15.0));
        assert_eq!(number("cm", "cm"), None);
        assert_eq!(parse_vowel("0.50 (a)"), Some(0.5));
        assert_eq!(parse_vowel("E"), Some(0.75));
        assert_eq!(parse_vowel("i"), Some(1.0));
        assert_eq!(parse_vowel("monk"), None);
    }

    #[test]
    fn guard_keeps_state_values_in_range() {
        assert_eq!(guard(0.25), Some(0.25));
        assert_eq!(guard(-1.0), Some(0.0));
        assert_eq!(guard(7.0), Some(1.0));
        assert_eq!(guard(f32::NAN), None);
        assert_eq!(guard(f32::INFINITY), None);
    }

    #[test]
    fn pad_forwards_while_held_and_on_a_change() {
        let rest = (0.0, 0.5);
        assert!(!pad_forward(None, false, rest), "first block after a reset");
        assert!(pad_forward(None, true, rest));
        assert!(!pad_forward(Some(rest), false, rest));
        assert!(
            pad_forward(Some(rest), false, (0.3, 0.5)),
            "a tap shorter than a block"
        );
        assert!(pad_forward(Some(rest), true, rest));
    }

    /// Keep these ids: `examples/write_presets.rs` names parameters by them.
    #[test]
    fn parameter_ids_are_stable() {
        let ids: Vec<String> = DejaLamaParams::default()
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(
            ids,
            [
                "porttime", "vowel", "delay", "headsize", "padgate", "padx", "pady"
            ]
        );
    }

    #[test]
    fn delta_saturates_at_i32_max() {
        assert_eq!(delta(0), 0);
        assert_eq!(delta(2047), 2047);
        assert_eq!(delta(u32::MAX), i32::MAX);
    }
}
