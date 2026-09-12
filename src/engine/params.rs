//! Define the DLL's parameters, their values and the factory programs.

/// Everything the DLL's `setParameter` accepts. Use `index()` for the DLL's VST numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    /// Index 0, "PortTime": glide time.
    PortTime,
    /// Index 1, "Vowel": also fed by the pitch-bend smoother.
    Vowel,
    /// Index 2, "Delay": delay level.
    Delay,
    /// Index 3, "HeadSize": formant scale.
    HeadSize,
    /// Index 5, GUI read-back: smoothed expression 0..1.
    Expression,
    /// Index 6, GUI read-back: mouth opening.
    Mouth,
    /// Index 9, GUI pad pressed (non-zero) or released (zero).
    PadGate,
    /// Index 10, GUI pad Y: the vowel, through the pitch-bend smoother.
    PadVowel,
    /// Index 11, GUI pad X: the pitch, through the expression smoother.
    PadPitch,
}

impl Param {
    /// The DLL's VST parameter index.
    #[must_use]
    pub const fn index(self) -> i32 {
        match self {
            Self::PortTime => 0,
            Self::Vowel => 1,
            Self::Delay => 2,
            Self::HeadSize => 3,
            Self::Expression => 5,
            Self::Mouth => 6,
            Self::PadGate => 9,
            Self::PadVowel => 10,
            Self::PadPitch => 11,
        }
    }

    /// The parameter at a DLL index, or `None` for the indices the DLL ignores.
    #[must_use]
    pub const fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::PortTime),
            1 => Some(Self::Vowel),
            2 => Some(Self::Delay),
            3 => Some(Self::HeadSize),
            5 => Some(Self::Expression),
            6 => Some(Self::Mouth),
            9 => Some(Self::PadGate),
            10 => Some(Self::PadVowel),
            11 => Some(Self::PadPitch),
            _ => None,
        }
    }

    /// Every parameter, in index order.
    pub const ALL: [Self; 9] = [
        Self::PortTime,
        Self::Vowel,
        Self::Delay,
        Self::HeadSize,
        Self::Expression,
        Self::Mouth,
        Self::PadGate,
        Self::PadVowel,
        Self::PadPitch,
    ];
}

/// Plugin parameters, each 0..1 (the original's VST parameter index in front).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Index 0, "PortTime": portamento; glide speed = 12 semitones per (`port_time` + 0.01) s.
    pub port_time: f32,
    /// Index 1, "Vowel": 0 = u, 0.25 = o, 0.5 = a, 0.75 = e, 1 = i.
    pub vowel: f32,
    /// Index 2, "Delay": delay send/feedback level (dB display).
    pub delay: f32,
    /// Index 3, "HeadSize": formant scale 0.75 + `0.5·head_size` (0.75x to 1.25x).
    pub head_size: f32,
}

impl Default for Params {
    fn default() -> Self {
        // Program 0 "Rabten"
        Params {
            port_time: 0.5,
            vowel: 0.5,
            delay: 0.8,
            head_size: 0.5,
        }
    }
}

/// A factory program. Programs carry no vowel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Program {
    pub name: &'static str,
    pub port_time: f32,
    pub delay: f32,
    pub head_size: f32,
}

impl Program {
    const fn new(name: &'static str, port_time: f32, delay: f32, head_size: f32) -> Self {
        Self {
            name,
            port_time,
            delay,
            head_size,
        }
    }
}

/// The five factory programs, in the DLL's order.
pub const PROGRAMS: [Program; 5] = [
    Program::new("Rabten", 0.5, 0.8, 0.5),
    Program::new("Dorje", 0.4, 0.3, 0.0),
    Program::new("Ngawang", 0.8, 0.6, 0.25),
    Program::new("Jamyang", 0.5, 0.0, 0.75),
    Program::new("Tinley", 1.0, 0.9, 1.0),
];

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values on purpose
mod tests {
    use super::*;

    #[test]
    fn indices_round_trip() {
        for param in Param::ALL {
            assert_eq!(Param::from_index(param.index()), Some(param));
        }
        for index in [-1, 4, 7, 8, 12] {
            assert_eq!(Param::from_index(index), None, "index {index}");
        }
    }

    #[test]
    fn defaults_are_program_zero() {
        let rabten = PROGRAMS[0];
        let defaults = Params::default();
        assert_eq!(rabten.name, "Rabten");
        assert_eq!(
            (defaults.port_time, defaults.delay, defaults.head_size),
            (rabten.port_time, rabten.delay, rabten.head_size)
        );
        assert_eq!(defaults.vowel, 0.5);
    }
}
