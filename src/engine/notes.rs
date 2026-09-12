//! Track the held keys in the DLL's last-note-priority stack.

/// Held-note stack depth.
const NOTE_STACK: usize = 128;

/// Held notes (MIDI note - 12), most recent first, 0 = empty.
pub(super) struct NoteStack([u8; NOTE_STACK]);

impl NoteStack {
    pub(super) const fn new() -> Self {
        Self([0; NOTE_STACK])
    }

    pub(super) fn clear(&mut self) {
        self.0 = [0; NOTE_STACK];
    }

    /// The note the voice plays, or 0.
    pub(super) const fn top(&self) -> u8 {
        self.0[0]
    }

    /// Whether a second key is down, which turns portamento on.
    pub(super) const fn has_second(&self) -> bool {
        self.0[1] != 0
    }

    /// Put a note in front. Shift the others up only while the stack has room, like the DLL.
    pub(super) fn push(&mut self, note: u8) {
        if self.0[NOTE_STACK - 1] == 0 {
            for k in (0..NOTE_STACK - 1).rev() {
                if self.0[k] != 0 {
                    self.0[k + 1] = self.0[k];
                }
            }
        }
        self.0[0] = note;
    }

    /// Remove a note by shifting the entries above it down. Keep the DLL's scan, which skips
    /// the entry that slides into a removed slot.
    pub(super) fn remove(&mut self, note: u8) {
        let mut k = 0usize;
        while k < NOTE_STACK {
            if self.0[k] == note {
                while self.0[k] != 0 {
                    self.0[k] = if k + 1 < NOTE_STACK { self.0[k + 1] } else { 0 };
                    k += 1;
                }
            }
            k += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_note_has_priority() {
        let mut stack = NoteStack::new();
        assert_eq!(stack.top(), 0);
        stack.push(48);
        stack.push(52);
        stack.push(55);
        assert_eq!((stack.top(), stack.has_second()), (55, true));
        stack.remove(52);
        assert_eq!(&stack.0[..3], &[55, 48, 0]);
        stack.remove(55);
        assert_eq!((stack.top(), stack.has_second()), (48, false));
        stack.remove(48);
        assert_eq!(stack.top(), 0);
    }

    /// The DLL's scan skips the entry that slides into a removed slot, so of two adjacent
    /// copies of a note only one goes.
    #[test]
    fn remove_skips_the_entry_that_slides_into_the_hole() {
        let mut stack = NoteStack::new();
        stack.push(40);
        stack.push(52);
        stack.push(52);
        stack.remove(52);
        assert_eq!(&stack.0[..3], &[52, 40, 0]);
        stack.remove(52);
        assert_eq!(&stack.0[..3], &[40, 0, 0]);
    }

    /// With the last slot in use, a push overwrites the top instead of shifting.
    #[test]
    fn push_stops_shifting_when_full() {
        let mut stack = NoteStack::new();
        for note in 1..=NOTE_STACK as u8 {
            stack.push(note);
        }
        assert_eq!((stack.0[0], stack.0[NOTE_STACK - 1]), (128, 1));
        stack.push(200);
        assert_eq!(
            (stack.0[0], stack.0[1], stack.0[NOTE_STACK - 1]),
            (200, 127, 1)
        );
    }
}
