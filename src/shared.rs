//! Values the audio thread publishes for the editor (the original called
//! `editor->setParameter()` from `processReplacing` for them).

use nice_plug::prelude::AtomicF32;

#[derive(Debug)]
pub struct GuiShared {
    /// Engine param 6: mouth opening; selects the monk animation frame.
    pub mouth: AtomicF32,
    /// Engine param 1: the vowel in use (pitch bend and pad Y move it); positions the vowel
    /// triangle.
    pub vowel: AtomicF32,
    /// Engine param 5: smoothed expression 0..1 (pad X / CC11); positions the pitch triangle.
    pub expression: AtomicF32,
}

impl Default for GuiShared {
    fn default() -> Self {
        Self {
            mouth: AtomicF32::new(0.1667),
            vowel: AtomicF32::new(0.5),
            expression: AtomicF32::new(0.0),
        }
    }
}
