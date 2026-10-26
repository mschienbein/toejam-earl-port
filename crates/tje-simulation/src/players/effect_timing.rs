//! Periods supplied by gameplay setup callers for eight-byte descriptors; timed
//! descriptors retain their imported period rather than a presenter heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectAnimation {
    TelephoneIdle,
    TelephoneRinging,
    Boombox,
    TomatoProjectile,
    ControlRemapLeft,
    ControlRemapRight,
    ShipCollection,
    Choice,
    StolenPresent,
    Pause,
    Decoy,
    RainCloud,
    Doorway,
    Rose,
    Transient,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationPeriod {
    Fixed(u16),
    ImportedDescriptor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectAnimationTiming {
    pub period: AnimationPeriod,
    pub axes: u8,
    pub frame_callback: bool,
}
pub fn effect_animation_timing(animation: EffectAnimation) -> EffectAnimationTiming {
    use AnimationPeriod::{Fixed, ImportedDescriptor};
    let period = match animation {
        // G/telephone_open.c -> TelephoneCreateEntity uses hold, then
        // TelephoneStartRinging installs the authored three-tick sequence.
        EffectAnimation::TelephoneIdle => Fixed(0),
        EffectAnimation::TelephoneRinging => Fixed(3),
        // G/present_effects.c OpenBoomBox sets frame_phase=2.
        EffectAnimation::Boombox => Fixed(2),
        // G/projectile_effects.c launch-remap sets frame_phase=9.
        EffectAnimation::ControlRemapLeft | EffectAnimation::ControlRemapRight => Fixed(9),
        // Explicit zero in projectile constructors; all other eight-byte users
        // inherit zero from ResetEntityRuntimeState's 47 cleared words.
        EffectAnimation::TomatoProjectile
        | EffectAnimation::ShipCollection
        | EffectAnimation::Choice
        | EffectAnimation::StolenPresent
        | EffectAnimation::Pause => Fixed(0),
        EffectAnimation::Decoy
        | EffectAnimation::RainCloud
        | EffectAnimation::Doorway
        | EffectAnimation::Rose
        | EffectAnimation::Transient => ImportedDescriptor,
    };
    EffectAnimationTiming {
        period,
        axes: 0,
        frame_callback: false,
    }
}
