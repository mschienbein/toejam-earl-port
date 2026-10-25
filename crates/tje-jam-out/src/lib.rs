//! Opt-in, presentation-only Jam Out gestures. Resources are resolved from the
//! user's cartridge; player animation, movement and collision state stay original.
#![forbid(unsafe_code)]

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use tje_mem::{Address, Bus, RenderPiece, Revision, ram, rom};

#[cfg(feature = "enhanced-mode")]
mod enhanced;
#[cfg(feature = "enhanced-mode")]
pub use enhanced::*;

#[cfg(any(feature = "enhanced-mode", test))]
const A: u8 = 0x40;
#[cfg(any(feature = "enhanced-mode", test))]
const B: u8 = 0x10;
#[cfg(any(feature = "enhanced-mode", test))]
const C: u8 = 0x20;
#[cfg(any(feature = "enhanced-mode", test))]
const ACTIONS: u8 = A | B | C;
#[cfg(any(feature = "enhanced-mode", test))]
const START: u8 = 0x80;

/// Session configuration, independent of ordinary three-button bindings.
/// Backing 0 keeps normal music; 1..=6 select the six existing Jam banks.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Encode, Decode, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub performance_sounds: bool,
    pub sounds_while_busy: bool,
    pub chord: bool,
    pub backing: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            performance_sounds: true,
            sounds_while_busy: false,
            chord: false,
            backing: 0,
        }
    }
}
impl Settings {
    pub fn valid(self) -> bool {
        self.backing <= 6
    }
}

#[derive(Clone, Copy, Debug, Default, Encode, Decode, PartialEq, Eq)]
enum Sequence {
    #[default]
    None,
    Bass,
    BassAlternate,
    Snare,
    SnareAlternate,
    Clap,
    Vocal,
    IdleTransition,
    IdleLoop,
}
impl Sequence {
    fn address(self, revision: Revision) -> u32 {
        match self {
            Self::None => 0,
            Self::Bass => rom::jam_out_character_animations(revision).address(),
            Self::BassAlternate => rom::jam_bass_drum_alternate_sequence(revision).address(),
            Self::Snare => rom::jam_snare_drum_first_sequence(revision).address(),
            Self::SnareAlternate => rom::jam_snare_drum_second_sequence(revision).address(),
            Self::Clap => rom::jam_clap_sequence(revision).address(),
            Self::Vocal => rom::jam_vocal_or_scratch_sequence(revision).address(),
            Self::IdleTransition => rom::jam_lead_idle_transition_sequence(revision).address(),
            Self::IdleLoop => rom::jam_lead_idle_loop_sequence(revision).address(),
        }
    }
    #[cfg(any(feature = "enhanced-mode", test))]
    fn period(self) -> u8 {
        match self {
            Self::Clap => 3,
            Self::Vocal => 2,
            _ => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Encode, Decode, PartialEq, Eq)]
enum Instrument {
    #[default]
    None,
    Bass,
    Snare,
    Clap,
    ToeJam,
    Earl,
    Rap,
    Jammin,
    Scratch,
}
impl Instrument {
    #[cfg(feature = "enhanced-mode")]
    fn sample(self, revision: Revision) -> Option<u32> {
        Some(match self {
            Self::None => return None,
            Self::Bass => rom::sfx_jam_bass_drum(revision).address(),
            Self::Snare => rom::sfx_jam_snare_drum(revision).address(),
            Self::Clap => rom::sfx_jam_clap(revision).address(),
            Self::ToeJam => rom::sfx_jam_toejam(revision).address(),
            Self::Earl => rom::sfx_jam_big_earl(revision).address(),
            Self::Rap => rom::sfx_jam_rap(revision).address(),
            Self::Jammin => rom::sfx_jammin(revision).address(),
            Self::Scratch => rom::sfx_jam_record_scratch(revision).address(),
        })
    }
    #[cfg(feature = "enhanced-mode")]
    fn fm_effect(self) -> Option<u32> {
        match self {
            Self::None => None,
            Self::Bass => Some(22),
            Self::Snare => Some(20),
            Self::Clap => Some(19),
            _ => Some(21),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Encode, Decode, PartialEq, Eq)]
struct Seat {
    previous: u8,
    suppressed: u8,
    chord_ticks: u8,
    held: bool,
    alternate: bool,
    locked: bool,
    sequence: Sequence,
    frame: i16,
    displayed_sequence: Sequence,
    displayed_frame: i16,
    timer: u8,
    pending: Instrument,
}
#[cfg(any(feature = "enhanced-mode", test))]
impl Seat {
    fn cancel(&mut self) {
        self.sequence = Sequence::None;
        self.displayed_sequence = Sequence::None;
        self.locked = false;
        self.pending = Instrument::None;
        self.timer = 0;
    }

    /// Called once after the original input filter, never by rendering. Keep a
    /// separate release latch so modifier/chord release cannot open a menu.
    fn input(&mut self, raw: u8, modifier: bool, eligible: bool, settings: Settings) -> u8 {
        self.suppressed &= raw;
        let both = raw & (B | C) == B | C;
        if settings.enabled && settings.chord && both && raw & START == 0 {
            self.chord_ticks = self.chord_ticks.saturating_add(1).min(10);
            self.suppressed |= B | C;
        } else {
            self.chord_ticks = 0;
        }
        let armed = settings.enabled && (modifier || self.chord_ticks == 10);
        let actions = if modifier { raw } else { raw & !(B | C) };
        let gesture = if armed && actions & ACTIONS != 0 && raw & START == 0 {
            actions & !START
        } else {
            0
        };
        let mut ordinary = raw & !self.suppressed;
        if settings.enabled && (armed || self.chord_ticks != 0) {
            let consumed = if raw & START != 0 {
                raw & ACTIONS
            } else if gesture != 0 {
                raw & !START
            } else {
                raw & (B | C)
            };
            self.suppressed |= consumed & ACTIONS;
            ordinary &= !consumed;
        }
        self.held = armed && ordinary & 0x0f == 0 && raw & START == 0;
        if !self.held || !eligible {
            self.cancel();
        }
        if gesture != 0 && gesture != self.previous {
            let instrument = if gesture & (A | B) == A | B {
                Instrument::Scratch
            } else if gesture & 1 != 0 {
                Instrument::ToeJam
            } else if gesture & 2 != 0 {
                Instrument::Earl
            } else if gesture & 4 != 0 {
                Instrument::Rap
            } else if gesture & 8 != 0 {
                Instrument::Jammin
            } else if gesture & C != 0 {
                Instrument::Clap
            } else if gesture & B != 0 {
                Instrument::Snare
            } else {
                Instrument::Bass
            };
            if settings.performance_sounds
                && (eligible && !self.locked || settings.sounds_while_busy)
            {
                self.pending = instrument;
            }
            if eligible && !self.locked {
                self.alternate = !self.alternate;
                self.sequence = match instrument {
                    Instrument::Bass if self.alternate => Sequence::Bass,
                    Instrument::Bass => Sequence::BassAlternate,
                    Instrument::Snare if self.alternate => Sequence::Snare,
                    Instrument::Snare => Sequence::SnareAlternate,
                    Instrument::Clap => Sequence::Clap,
                    _ => Sequence::Vocal,
                };
                self.locked = matches!(self.sequence, Sequence::Clap | Sequence::Vocal);
                self.frame = -1;
                self.timer = 1;
            }
        }
        self.previous = if eligible || settings.sounds_while_busy {
            gesture
        } else {
            0
        };
        ordinary
    }

    fn animate<B: Bus + ?Sized>(&mut self, bus: &B, revision: Revision) {
        if self.sequence == Sequence::None {
            return;
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer != 0 {
            return;
        }
        let next = self.frame + 1;
        let entry = bus.read_u8(self.sequence.address(revision) + next as u32) as i8;
        match entry {
            -1 => self.frame = 0,
            -2 => {
                self.sequence = if self.sequence == Sequence::IdleTransition {
                    Sequence::IdleLoop
                } else {
                    Sequence::IdleTransition
                };
                self.frame = -1;
                self.timer = 1;
                self.locked = false;
                return;
            }
            _ => self.frame = next,
        }
        self.displayed_sequence = self.sequence;
        self.displayed_frame = self.frame;
        self.timer = self.sequence.period();
    }
}

/// All authoritative state is fixed-capacity and serializable. No ROM pointer,
/// renderer cache, original RNG draw, wall clock or mutable singleton lives here.
#[derive(Clone, Debug, Default, Encode, Decode, PartialEq, Eq)]
pub struct State {
    settings: Settings,
    requested: Settings,
    seats: [Seat; 2],
    in_play: bool,
    suspended: u8,
    pcm_cooldown: u8,
    audio_frame: Option<u64>,
    applied_backing: u8,
    pub configuration_events: u64,
    pub accepted_pcm: u64,
    pub accepted_fm: u64,
    pub dropped_requests: u64,
}
impl State {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            requested: settings,
            ..Self::default()
        }
    }
    pub fn requested_settings(&self) -> Settings {
        self.requested
    }
    pub fn active_poses(&self) -> [bool; 2] {
        self.seats.map(|seat| seat.sequence != Sequence::None)
    }
    #[cfg(any(feature = "enhanced-mode", test))]
    fn apply_requested(&mut self) {
        if self.settings != self.requested {
            self.settings = self.requested;
            self.configuration_events = self.configuration_events.wrapping_add(1);
            for seat in &mut self.seats {
                seat.cancel();
            }
        }
    }
    #[cfg(any(feature = "enhanced-mode", test))]
    fn cancel(&mut self) {
        for seat in &mut self.seats {
            seat.cancel();
        }
    }

    #[cfg(any(feature = "enhanced-mode", test))]
    fn arbitrate(
        &mut self,
        frame: u64,
        permitted: bool,
        pcm_idle: bool,
        fm_idle: bool,
    ) -> [Instrument; 2] {
        self.apply_requested();
        if self.audio_frame == Some(frame) {
            return [Instrument::None; 2];
        }
        self.audio_frame = Some(frame);
        self.pcm_cooldown = self.pcm_cooldown.saturating_sub(1);
        let requested = self.seats.map(|seat| seat.pending);
        for seat in &mut self.seats {
            seat.pending = Instrument::None;
        }
        let mut accepted = [Instrument::None; 2];
        if permitted && self.settings.enabled && self.settings.performance_sounds {
            if requested[0] != Instrument::None && pcm_idle && self.pcm_cooldown == 0 {
                accepted[0] = requested[0];
                self.pcm_cooldown = 6;
                self.accepted_pcm += 1;
            }
            if requested[1] != Instrument::None && fm_idle {
                accepted[1] = requested[1];
                self.accepted_fm += 1;
            }
        }
        for seat in 0..2 {
            self.dropped_requests += u64::from(
                requested[seat] != Instrument::None && accepted[seat] == Instrument::None,
            );
        }
        accepted
    }
}

/// Conservative eligibility: only ordinary, grounded, stationary idle players.
/// The presentation never modifies these conditions or restores saved velocity.
pub fn eligible<B: Bus + ?Sized>(bus: &B, revision: Revision, seat: i32) -> bool {
    let actor = ram::g_player_info(revision).at(seat);
    let terrain = if seat == 0 {
        ram::g_player1_terrain_underfoot(revision)
    } else {
        ram::g_player2_terrain_underfoot(revision)
    }
    .read(bus)
        & 0xf0;
    ram::g_player_lives(revision).at(seat).read(bus) >= 0
        && ram::g_player_health(revision).at(seat).read(bus) > 0
        && ram::g_player_inventory_open(revision).at(seat).read(bus) == 0
        && ram::g_player_elevator_flags(revision).at(seat).read(bus) == 0
        && ram::g_player_powerups(revision).at(seat).read(bus) == -1
        && ram::g_player_fall_states(revision).at(seat).read(bus) == 0
        && matches!(terrain, 0x10 | 0x30 | 0x40 | 0x50)
        && actor.draw_flags().read(bus) & 0x89 == 1
        && actor.behavior_flags().read(bus) & 0x20 == 0
        && actor.sprite_state().read(bus) == 0
        && actor.height().read(bus) == 0
        && actor.velocity_x().read(bus) == 0
        && actor.velocity_y().read(bus) == 0
        && actor.velocity_height().read(bus) == 0
        && actor.acceleration_x().read(bus) == 0
        && actor.acceleration_y().read(bus) == 0
        && actor.acceleration_height().read(bus) == 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presentation {
    pub entity: u8,
    pub frame_address: u32,
    pub horizontal_flip: bool,
    pub palette_index: u8,
}

impl State {
    /// Read-only: repeated draws cannot consume input or advance an animation.
    pub fn presentation<B: Bus + ?Sized>(
        &self,
        bus: &B,
        revision: Revision,
    ) -> [Option<Presentation>; 2] {
        std::array::from_fn(|index| {
            let seat = self.seats[index];
            if !self.settings.enabled
                || !self.in_play
                || self.suspended != 0
                || seat.displayed_sequence == Sequence::None
                || seat.displayed_frame < 0
                || !eligible(bus, revision, index as i32)
            {
                return None;
            }
            let entry = bus
                .read_u8(seat.displayed_sequence.address(revision) + seat.displayed_frame as u32);
            let frames = if index == 0 {
                rom::spr_jamout_toejam(revision)
            } else {
                rom::spr_jamout_earl(revision)
            };
            Some(Presentation {
                entity: index as u8,
                frame_address: frames.at(i32::from(entry & 0x7f)).read(bus).address(),
                horizontal_flip: entry & 0x80 != 0,
                palette_index: {
                    let canonical = ram::g_player_info(revision)
                        .at(index as i32)
                        .current_frame()
                        .read(bus)
                        .address();
                    Address::<RenderPiece>::new(canonical + 4)
                        .tile_format()
                        .read(bus) as u8
                        & 3
                },
            })
        })
    }
}

#[cfg(test)]
mod tests;
