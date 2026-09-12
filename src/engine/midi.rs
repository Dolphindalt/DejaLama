//! Decode the MIDI messages the DLL reacts to, and keep its workaround for hosts that stamp
//! every event with sample offset 0.

/// The MIDI messages the DLL reacts to. Data bytes carry 7-bit values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMessage {
    /// Velocity 0 acts as a note off, as in the DLL.
    NoteOn {
        note: u8,
        velocity: u8,
    },
    NoteOff {
        note: u8,
    },
    ControlChange {
        controller: u8,
        value: u8,
    },
    /// 14-bit bend as its two data bytes; the DLL reads it as `msb * 128 + lsb`.
    PitchBend {
        lsb: u8,
        msb: u8,
    },
}

impl MidiMessage {
    /// Decode a raw 3-byte message the way the DLL's event queue does: match on the status
    /// nibble, mask the data bytes to 7 bits, and return `None` for statuses the DLL ignores.
    #[must_use]
    pub const fn from_bytes(status: u8, data1: u8, data2: u8) -> Option<Self> {
        let (data1, data2) = (data1 & 0x7f, data2 & 0x7f);
        match status & 0xf0 {
            0x80 => Some(Self::NoteOff { note: data1 }),
            0x90 => Some(Self::NoteOn {
                note: data1,
                velocity: data2,
            }),
            0xb0 => Some(Self::ControlChange {
                controller: data1,
                value: data2,
            }),
            0xe0 => Some(Self::PitchBend {
                lsb: data1,
                msb: data2,
            }),
            _ => None,
        }
    }
}

/// The controllers the DLL handles. Use `number()` for the MIDI controller number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlChange {
    /// CC1: vibrato depth and rate.
    ModWheel,
    /// CC5: glide time (param 0).
    Portamento,
    /// CC7: output volume, `value * 0.001`.
    Volume,
    /// CC11: pitch, MIDI note 36 + 12 * value / 127 through the expression smoother.
    Expression,
    /// CC12: delay level (param 2).
    DelayLevel,
    /// CC13: head size (param 3).
    HeadSize,
}

impl ControlChange {
    /// The MIDI controller number.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::ModWheel => 1,
            Self::Portamento => 5,
            Self::Volume => 7,
            Self::Expression => 11,
            Self::DelayLevel => 12,
            Self::HeadSize => 13,
        }
    }

    /// The controller for a MIDI controller number, or `None` for the ones the DLL ignores.
    #[must_use]
    pub const fn from_number(number: u8) -> Option<Self> {
        match number {
            1 => Some(Self::ModWheel),
            5 => Some(Self::Portamento),
            7 => Some(Self::Volume),
            11 => Some(Self::Expression),
            12 => Some(Self::DelayLevel),
            13 => Some(Self::HeadSize),
            _ => None,
        }
    }

    /// Every handled controller.
    pub const ALL: [Self; 6] = [
        Self::ModWheel,
        Self::Portamento,
        Self::Volume,
        Self::Expression,
        Self::DelayLevel,
        Self::HeadSize,
    ];
}

/// A queued MIDI message with its sample offset inside the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiEvent {
    pub delta_frames: i32,
    pub message: MidiMessage,
}

/// Pitch bends the delta-0 spreading hack can hold per block.
const SPREAD_CAPACITY: usize = 1024;

/// The DLL's workaround for hosts that stamp every event with offset 0: pitch bends that arrive
/// at sample 0 apply one by one, `(n - 2) / count` samples apart, starting at sample 1.
pub(super) struct BendSpread {
    /// the bends collected at sample 0, as (lsb, msb)
    bends: Vec<(u8, u8)>,
    /// block sample at which the next bend applies
    next: i32,
    /// bends in this block's set
    count: i32,
    /// samples between two bends
    interval: i32,
    /// the next bend to apply
    index: i32,
}

impl BendSpread {
    pub(super) fn new() -> Self {
        Self {
            bends: Vec::with_capacity(SPREAD_CAPACITY),
            next: 0,
            count: 0,
            interval: 0,
            index: 0,
        }
    }

    /// Store a bend that arrived at sample 0; `first` starts a new set for this block.
    pub(super) fn collect(&mut self, first: bool, bend: (u8, u8)) -> bool {
        if first {
            self.bends.clear();
        }
        if self.bends.len() < SPREAD_CAPACITY {
            self.bends.push(bend);
            true
        } else {
            false
        }
    }

    /// Schedule the bends collected at sample 0 over a block of `n` samples.
    pub(super) fn schedule(&mut self, count: i32, n: i32) {
        self.next = 1;
        self.count = count;
        self.index = 0;
        self.interval = (n - 2) / count;
    }

    /// The bend due at sample `i`, if any.
    pub(super) fn due(&mut self, i: i32) -> Option<(u8, u8)> {
        if i == self.next && i != 0 && self.index < self.count {
            let bend = self.bends[self.index as usize];
            self.index += 1;
            self.next += self.interval;
            Some(bend)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_bytes_decodes_the_four_handled_statuses() {
        assert_eq!(
            MidiMessage::from_bytes(0x90, 60, 100),
            Some(MidiMessage::NoteOn {
                note: 60,
                velocity: 100
            })
        );
        // the channel nibble and the high data bits fall away
        assert_eq!(
            MidiMessage::from_bytes(0x83, 0x80 | 0x3c, 0),
            Some(MidiMessage::NoteOff { note: 60 })
        );
        assert_eq!(
            MidiMessage::from_bytes(0xb0, 1, 127),
            Some(MidiMessage::ControlChange {
                controller: 1,
                value: 127
            })
        );
        assert_eq!(
            MidiMessage::from_bytes(0xe0, 0, 64),
            Some(MidiMessage::PitchBend { lsb: 0, msb: 64 })
        );
        for status in [0xa0, 0xc0, 0xd0, 0xf0] {
            assert_eq!(MidiMessage::from_bytes(status, 0, 0), None, "{status:#x}");
        }
    }

    #[test]
    fn controller_numbers_round_trip() {
        for cc in ControlChange::ALL {
            assert_eq!(ControlChange::from_number(cc.number()), Some(cc));
        }
        for number in [0, 2, 10, 64, 74, 120] {
            assert_eq!(ControlChange::from_number(number), None, "CC {number}");
        }
    }

    /// Three bends collected at sample 0 of a 64-sample block apply at samples 1, 21 and 41.
    #[test]
    fn spread_schedules_bends_across_the_block() {
        let mut spread = BendSpread::new();
        assert!(spread.collect(true, (0, 0)));
        assert!(spread.collect(false, (0, 32)));
        assert!(spread.collect(false, (0, 64)));
        spread.schedule(3, 64);
        let due: Vec<(i32, (u8, u8))> = (0..64)
            .filter_map(|i| spread.due(i).map(|bend| (i, bend)))
            .collect();
        assert_eq!(due, [(1, (0, 0)), (21, (0, 32)), (41, (0, 64))]);
    }

    /// A one-sample block gives an interval of -1; the bend then waits for sample 1 of a later
    /// block, because the schedule persists.
    #[test]
    fn spread_keeps_its_schedule_across_blocks() {
        let mut spread = BendSpread::new();
        assert!(spread.collect(true, (1, 2)));
        spread.schedule(1, 1);
        assert_eq!(spread.interval, -1);
        assert_eq!(spread.due(0), None);
        assert_eq!(spread.due(1), Some((1, 2)));
        assert_eq!(spread.due(1), None);
    }

    #[test]
    fn spread_drops_bends_beyond_its_capacity() {
        let mut spread = BendSpread::new();
        assert!(spread.collect(true, (0, 0)));
        for _ in 1..SPREAD_CAPACITY {
            assert!(spread.collect(false, (0, 1)));
        }
        assert!(!spread.collect(false, (0, 2)));
        assert_eq!(spread.bends.len(), SPREAD_CAPACITY);
    }
}
