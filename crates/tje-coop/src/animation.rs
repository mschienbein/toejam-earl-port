//! Simulation-owned program admissions. Presentation only reads saved selections.
use crate::publication::resources::{ResourceCatalog, ResourceError};
use tje_mode_view::animation::{Cursor, Step};
use tje_simulation::{
    SeatId,
    players::{PlayerAnimationStep, PlayerServices},
    population::{AnimationCursor, AnimationSample, Kind},
    visual::Playback,
};

fn resource_error(error: ResourceError) -> tje_world::Error {
    match error {
        ResourceError::Invalid(reason) => tje_world::Error::Resource(reason),
        ResourceError::Definition(error) => error,
        ResourceError::View(tje_mode_view::Error::Invalid(reason)) => {
            tje_world::Error::Resource(reason)
        }
        ResourceError::View(_) | ResourceError::Decoder(_) | ResourceError::Contract(_) => {
            tje_world::Error::Resource("invalid checked animation resource or contract")
        }
    }
}
fn view_error(error: tje_mode_view::Error) -> tje_world::Error {
    match error {
        tje_mode_view::Error::Invalid(reason) => tje_world::Error::State(reason),
        tje_mode_view::Error::Resource(_) => {
            tje_world::Error::Resource("invalid checked frame data")
        }
    }
}

pub fn population_step(
    resources: &ResourceCatalog,
    kind: Kind,
    state: u8,
    cursor: &mut AnimationCursor,
) -> tje_world::Result<AnimationSample> {
    let id = resources.earthling(kind, state).map_err(resource_error)?;
    let animation = resources.program(id).map_err(resource_error)?;
    let period = animation.period.ok_or(tje_world::Error::Resource(
        "Earthling animation has no admitted timing",
    ))?;
    let mut next = Cursor {
        sequence_index: cursor.sequence_index,
        ticks_left: cursor.ticks_left,
        held: cursor.held,
        flip_horizontal: cursor.flip_horizontal,
        started: cursor.started,
    };
    let step = if next.started {
        animation
            .program
            .step(&mut next, period)
            .map_err(view_error)?
    } else {
        let (initial, step) = animation.program.start(period).map_err(view_error)?;
        next = initial;
        step
    };
    *cursor = AnimationCursor {
        sequence_index: next.sequence_index,
        ticks_left: next.ticks_left,
        held: next.held,
        flip_horizontal: next.flip_horizontal,
        started: next.started,
    };
    Ok(AnimationSample {
        frame: step.frame,
        motion: step.motion.map(|motion| [motion.x, motion.y]),
        finished: step.held,
    })
}

pub fn step_playback(
    resources: &ResourceCatalog,
    cursor: &mut Playback,
) -> tje_world::Result<Step> {
    resources
        .contracts()
        .validate_cursor(cursor)
        .map_err(|_| tje_world::Error::State("invalid saved animation continuation"))?;
    let animation = resources
        .program(cursor.animation)
        .map_err(resource_error)?;
    if animation
        .period
        .is_some_and(|period| period != cursor.period)
    {
        return Err(tje_world::Error::State(
            "logical animation timing differs from imported descriptor",
        ));
    }
    let mut next = Cursor {
        sequence_index: cursor.sequence_index,
        ticks_left: cursor.ticks_left,
        held: cursor.held,
        flip_horizontal: cursor.flip_horizontal,
        started: cursor.started,
    };
    let step = if next.started {
        animation
            .program
            .step(&mut next, cursor.period)
            .map_err(view_error)?
    } else {
        let (initial, step) = animation.program.start(cursor.period).map_err(view_error)?;
        next = initial;
        step
    };
    cursor.sequence_index = next.sequence_index;
    cursor.ticks_left = next.ticks_left;
    cursor.held = next.held;
    cursor.flip_horizontal = next.flip_horizontal;
    cursor.started = next.started;
    Ok(step)
}

pub fn hero_step(
    resources: &ResourceCatalog,
    coop: &tje_four_player::CoopState,
    services: &PlayerServices,
    seat: SeatId,
) -> tje_world::Result<PlayerAnimationStep> {
    seat.validate()
        .map_err(|_| tje_world::Error::State("invalid hero animation seat"))?;
    let player = &coop.players[seat.index()];
    let runtime = &services.runtime[seat.index()];
    let id = resources
        .hero(player.character.archetype, player.animation.descriptor)
        .map_err(resource_error)?;
    let animation = resources.program(id).map_err(resource_error)?;
    let period = animation.period.ok_or(tje_world::Error::Resource(
        "hero animation has no admitted timing",
    ))?;
    let started =
        runtime.animation.started && runtime.animation.descriptor == player.animation.descriptor;
    let mut cursor = Cursor {
        sequence_index: player.animation.sequence_index,
        ticks_left: u16::from(player.animation.ticks_left),
        held: runtime.animation.held,
        flip_horizontal: player.animation.flip_horizontal,
        started,
    };
    let step = if cursor.started {
        animation
            .program
            .step(&mut cursor, period)
            .map_err(view_error)?
    } else {
        let (initial, step) = animation.program.start(period).map_err(view_error)?;
        cursor = initial;
        step
    };
    Ok(PlayerAnimationStep {
        descriptor: player.animation.descriptor,
        sequence_index: cursor.sequence_index,
        ticks_left: u8::try_from(cursor.ticks_left)
            .map_err(|_| tje_world::Error::State("hero animation byte timer"))?,
        flip_horizontal: cursor.flip_horizontal,
        held: cursor.held,
        motion: step.motion.map(|motion| [motion.x, motion.y]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "explicit private-ROM animation admission; not a gameplay session"]
    fn both_revisions_admit_every_hero_pose_in_every_seat_without_render_visits() {
        for revision in tje_rom::Revision::ALL {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../roms")
                .join(match revision {
                    tje_rom::Revision::Rev00 => "rev00.bin",
                    tje_rom::Revision::Rev02 => "rev02.bin",
                });
            let rom = tje_rom::Rom::import(&std::fs::read(path).unwrap()).unwrap();
            let memory = tje_mem::AddressSpace::new(rom.into_bytes(), revision);
            let resources = ResourceCatalog::load(&memory).unwrap();
            for archetype in [
                tje_four_player::Archetype::ToeJam,
                tje_four_player::Archetype::Earl,
            ] {
                let mut config = tje_four_player::Config::four_seats(0);
                for character in &mut config.roster {
                    character.archetype = archetype;
                }
                let mut coop = tje_four_player::CoopState::new(config).unwrap();
                let mut services =
                    PlayerServices::new(0, tje_simulation::resources::ResourceLimits::default())
                        .unwrap();
                for seat in SeatId::ALL {
                    for descriptor in 0..121 {
                        coop.players[seat.index()].animation.descriptor = descriptor;
                        services.runtime[seat.index()].animation =
                            tje_simulation::players::AnimationPlayback {
                                descriptor: (descriptor + 1) % 121,
                                started: true,
                                held: true,
                            };
                        let id = resources.hero(archetype, descriptor).unwrap();
                        let imported = resources.program(id).unwrap();
                        let period = imported.period.unwrap();
                        let (mut expected, first) = imported.program.start(period).unwrap();
                        let sample = hero_step(&resources, &coop, &services, seat).unwrap();
                        assert_eq!(sample.sequence_index, expected.sequence_index);
                        assert_eq!(u16::from(sample.ticks_left), expected.ticks_left);
                        assert_eq!(
                            sample.motion,
                            first.motion.map(|motion| [motion.x, motion.y])
                        );
                        let mut saved = Playback::new(id, period);
                        assert_eq!(step_playback(&resources, &mut saved).unwrap(), first);
                        for _ in 0..32 {
                            let next = imported.program.step(&mut expected, period).unwrap();
                            assert_eq!(step_playback(&resources, &mut saved).unwrap(), next);
                            assert_eq!(saved.sequence_index, expected.sequence_index);
                            assert_eq!(saved.ticks_left, expected.ticks_left);
                        }
                    }
                }
            }
        }
    }
}
