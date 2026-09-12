//! Replay a text script through the engine. Use the same script format to drive the original
//! DLL through a minimal VST2 host and compare both outputs sample for sample; the regression
//! tests in `tests/engine_regression.rs` pin the hashes of such comparisons.
//!
//! Commands, one per line (`#` starts a comment):
//!
//! * `sr <hz>`: the sample rate the next `open` uses (44100); `block <n>`: the block size `run`
//!   uses (512)
//! * `open`: create the engine
//! * `nospread`: turn the delta-0 bend spreading off; leave it out of scripts meant for the DLL,
//!   which always spreads (the VST2 host ignores the command)
//! * `setsr <hz>`: store a new sample rate for the next `resume` to apply
//! * `resume`: reset the engine (the host's `effMainsChanged`)
//! * `printparams`: do nothing; the VST2 host prints the parameters
//! * `program <i>`: load a factory program
//! * `param <i> <v>`: set a parameter by its DLL index
//! * `midi <delta> <status> <d1> <d2>`: queue a MIDI message for the next `run`
//! * `run <blocks>`: render blocks; the queued events go to the first one

use crate::engine::{DelayLama, MidiEvent, MidiMessage, Param};
use std::fmt;
use std::str::FromStr;

#[derive(Debug)]
pub enum ScriptError {
    UnknownCommand(String),
    BadNumber {
        command: &'static str,
        field: &'static str,
        message: String,
    },
    NotOpen(&'static str),
    UnknownParam(i32),
    UnknownStatus(u8),
    /// A block longer than the engine's ring allows at the current sample rate.
    BlockTooLong {
        block: usize,
        max: usize,
    },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand(line) => write!(f, "unknown command: {line}"),
            Self::BadNumber {
                command,
                field,
                message,
            } => write!(f, "`{command}`: bad {field}: {message}"),
            Self::NotOpen(command) => write!(f, "`{command}` before `open`"),
            Self::UnknownParam(index) => write!(f, "`param`: the DLL has no parameter {index}"),
            Self::UnknownStatus(status) => {
                write!(f, "`midi`: the DLL ignores status {status:#04x}")
            }
            Self::BlockTooLong { block, max } => {
                write!(
                    f,
                    "`run`: block {block} exceeds the {max} samples the ring allows"
                )
            }
        }
    }
}

/// A script error with the 1-based line it came from.
#[derive(Debug)]
pub struct LineError {
    pub line: usize,
    pub error: ScriptError,
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.error)
    }
}

impl std::error::Error for ScriptError {}
impl std::error::Error for LineError {}

/// One script line.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    SampleRate(f32),
    BlockSize(usize),
    Open,
    NoSpread,
    SetSampleRate(f32),
    Resume,
    PrintParams,
    Program(usize),
    Param(Param, f32),
    Midi(MidiEvent),
    Run(usize),
}

/// Parse one numeric field and name it in the error.
fn num<T: FromStr>(text: &str, command: &'static str, field: &'static str) -> Result<T, ScriptError>
where
    T::Err: fmt::Display,
{
    text.parse().map_err(|e: T::Err| ScriptError::BadNumber {
        command,
        field,
        message: e.to_string(),
    })
}

/// Parse a finite value in a range; the engine indexes tables with these.
fn bounded(
    text: &str,
    command: &'static str,
    field: &'static str,
    range: std::ops::RangeInclusive<f32>,
) -> Result<f32, ScriptError> {
    let value: f32 = num(text, command, field)?;
    if value.is_finite() && range.contains(&value) {
        Ok(value)
    } else {
        Err(ScriptError::BadNumber {
            command,
            field,
            message: format!("{value} lies outside {}..={}", range.start(), range.end()),
        })
    }
}

/// The sample rates the engine's tables make sense at.
const SAMPLE_RATES: std::ops::RangeInclusive<f32> = 1000.0..=1_000_000.0;

impl FromStr for Command {
    type Err = ScriptError;

    fn from_str(line: &str) -> Result<Self, ScriptError> {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        Ok(match tokens.as_slice() {
            ["sr", hz] => Self::SampleRate(bounded(hz, "sr", "rate", SAMPLE_RATES)?),
            ["block", size] => Self::BlockSize(num(size, "block", "size")?),
            ["open"] => Self::Open,
            ["nospread"] => Self::NoSpread,
            ["setsr", hz] => Self::SetSampleRate(bounded(hz, "setsr", "rate", SAMPLE_RATES)?),
            ["resume"] => Self::Resume,
            ["printparams"] => Self::PrintParams,
            ["program", index] => Self::Program(num(index, "program", "index")?),
            ["param", index, value] => {
                let index = num::<i32>(index, "param", "index")?;
                Self::Param(
                    Param::from_index(index).ok_or(ScriptError::UnknownParam(index))?,
                    bounded(value, "param", "value", 0.0..=1.0)?,
                )
            }
            ["midi", delta, status, data1, data2] => {
                let status: u8 = num(status, "midi", "status")?;
                let message = MidiMessage::from_bytes(
                    status,
                    num(data1, "midi", "data1")?,
                    num(data2, "midi", "data2")?,
                )
                .ok_or(ScriptError::UnknownStatus(status))?;
                Self::Midi(MidiEvent {
                    delta_frames: num(delta, "midi", "delta")?,
                    message,
                })
            }
            ["run", blocks] => Self::Run(num(blocks, "run", "block count")?),
            _ => return Err(ScriptError::UnknownCommand(line.to_string())),
        })
    }
}

/// Parse a whole script into numbered commands. Skip blank lines and `#` comments.
fn parse(script: &str) -> Result<Vec<(usize, Command)>, LineError> {
    script
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .map(|(line, text)| {
            text.parse()
                .map(|command| (line, command))
                .map_err(|error| LineError { line, error })
        })
        .collect()
}

/// The interpreter state: settings, the engine once `open` ran, the events queued for the
/// next `run`, and the interleaved stereo output so far.
struct Session {
    sample_rate: f32,
    block: usize,
    synth: Option<DelayLama>,
    queue: Vec<MidiEvent>,
    out: Vec<f32>,
}

/// Borrow the engine, or report the command that ran before `open`.
fn require_open<'a>(
    synth: &'a mut Option<DelayLama>,
    command: &'static str,
) -> Result<&'a mut DelayLama, ScriptError> {
    synth.as_mut().ok_or(ScriptError::NotOpen(command))
}

impl Session {
    fn apply(&mut self, command: Command) -> Result<(), ScriptError> {
        match command {
            Command::SampleRate(hz) => self.sample_rate = hz,
            Command::BlockSize(size) => self.block = size,
            Command::Open => self.synth = Some(DelayLama::new(self.sample_rate)),
            Command::NoSpread => {
                require_open(&mut self.synth, "nospread")?.set_spread_pitch_bends(false);
            }
            Command::SetSampleRate(hz) => {
                self.sample_rate = hz;
                require_open(&mut self.synth, "setsr")?.set_sample_rate_deferred(hz);
            }
            Command::Resume => require_open(&mut self.synth, "resume")?.reset(),
            Command::PrintParams => {}
            Command::Program(index) => {
                require_open(&mut self.synth, "program")?.set_program(index);
            }
            Command::Param(param, value) => {
                require_open(&mut self.synth, "param")?.set_parameter(param, value);
            }
            Command::Midi(event) => self.queue.push(event),
            Command::Run(blocks) => {
                let synth = require_open(&mut self.synth, "run")?;
                let max = synth.max_block_len();
                if self.block > max {
                    return Err(ScriptError::BlockTooLong {
                        block: self.block,
                        max,
                    });
                }
                let (mut left, mut right) = (vec![0f32; self.block], vec![0f32; self.block]);
                // Deliver the queued events to the first block only, in offset order. Keep the
                // stable sort: events with equal offsets stay in script order, like the DLL's
                // queue.
                self.queue.sort_by_key(|event| event.delta_frames);
                for _ in 0..blocks {
                    synth.process(&self.queue, &mut left, &mut right);
                    self.queue.clear();
                    self.out
                        .extend(left.iter().zip(&right).flat_map(|(&l, &r)| [l, r]));
                }
            }
        }
        Ok(())
    }
}

/// Run a script and return its interleaved stereo output.
///
/// # Errors
///
/// Report the first line that fails to parse or runs before `open`.
pub fn run(script: &str) -> Result<Vec<f32>, LineError> {
    let mut session = Session {
        sample_rate: 44100.0,
        block: 512,
        synth: None,
        queue: Vec::new(),
        out: Vec::new(),
    };
    for (line, command) in parse(script)? {
        session
            .apply(command)
            .map_err(|error| LineError { line, error })?;
    }
    Ok(session.out)
}

/// FNV-1a over the little-endian bytes of the samples: the fingerprint the regression tests
/// compare with the DLL's output.
#[must_use]
pub fn fnv1a(samples: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in samples.iter().flat_map(|s| s.to_le_bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command() {
        let script = "sr 48000\nblock 64\nopen\nnospread\nsetsr 44100\nresume\nprintparams\n\
                      program 2\nparam 3 0.25\nmidi 5 144 60 100\nrun 3\n";
        let commands: Vec<Command> = parse(script)
            .unwrap()
            .into_iter()
            .map(|(_, command)| command)
            .collect();
        assert_eq!(
            commands,
            [
                Command::SampleRate(48000.0),
                Command::BlockSize(64),
                Command::Open,
                Command::NoSpread,
                Command::SetSampleRate(44100.0),
                Command::Resume,
                Command::PrintParams,
                Command::Program(2),
                Command::Param(Param::HeadSize, 0.25),
                Command::Midi(MidiEvent {
                    delta_frames: 5,
                    message: MidiMessage::NoteOn {
                        note: 60,
                        velocity: 100,
                    },
                }),
                Command::Run(3),
            ]
        );
    }

    #[test]
    fn errors_name_the_line() {
        let err = run("sr 44100\n\n# comment\nmidi 0 144 60 1x\n").unwrap_err();
        assert_eq!(err.line, 4);
        assert!(
            matches!(
                err.error,
                ScriptError::BadNumber {
                    command: "midi",
                    field: "data2",
                    ..
                }
            ),
            "{err}"
        );

        let err = run("param 0 0.5\n").unwrap_err();
        assert_eq!(err.line, 1);
        assert!(matches!(err.error, ScriptError::NotOpen("param")), "{err}");

        let err = run("open\nparam 4 0.5\n").unwrap_err();
        assert!(matches!(err.error, ScriptError::UnknownParam(4)), "{err}");

        let err = run("open\nmidi 0 160 60 1\n").unwrap_err();
        assert!(
            matches!(err.error, ScriptError::UnknownStatus(160)),
            "{err}"
        );

        let err = run("open\nfrobnicate 3\n").unwrap_err();
        assert!(matches!(err.error, ScriptError::UnknownCommand(_)), "{err}");

        for script in ["sr 100\n", "open\nparam 3 NaN\n", "open\nparam 3 1.5\n"] {
            let err = run(script).unwrap_err();
            assert!(matches!(err.error, ScriptError::BadNumber { .. }), "{err}");
        }
        let err = run("sr 192000\nblock 8192\nopen\nrun 1\n").unwrap_err();
        assert!(
            matches!(
                err.error,
                ScriptError::BlockTooLong {
                    block: 8192,
                    max: 6400
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn run_renders_the_requested_frames() {
        let out = run("sr 44100\nblock 100\nopen\nrun 3\n").unwrap();
        assert_eq!(out.len(), 600);
        assert!(out.iter().all(|&s| s == 0.0), "an idle voice stays silent");
    }

    #[test]
    fn fnv1a_matches_the_reference_values() {
        assert_eq!(fnv1a(&[]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(&[0.0]), 0x4d25_767f_9dce_13f5); // four zero bytes
    }
}
