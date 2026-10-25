use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ElevatorRecord, ObjectState, ram};

fn magnitude(context: &mut dyn Context, difference: i32) -> Result<i32> {
    Ok(call(context, ids::ABS, &[difference as u32])? as i32)
}

/// G/src/elevator/elevator_scene.c:240 (`ElevatorBlocksPlayerMovement`).
/// Preserve type-first record iteration, narrowed distances, and flag stores.
pub fn elevator_blocks_player_movement(
    context: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player_number = args[0] as i16;
    let x = args[1] as i16;
    let y = args[2] as i16;
    let elevation = args[3] as i16;
    let player = ram::g_player_info(revision)
        .cast::<ObjectState>()
        .byte_offset(i32::from(player_number.wrapping_shl(7)));
    let flag = ram::g_player_elevator_flags(revision).at(i32::from(player_number));
    for kind in 0..2 {
        for bank in 0..2 {
            let handle = (bank * 2 + kind + 1) as i8;
            let elevator = ram::g_elevator_records(revision).at(bank).at(kind);
            if elevator.state().read(context) != 11
                && elevator.state().read(context) != 13
                && player.level().read(context) == elevator.level().read(context)
            {
                let difference = i32::from(x) - i32::from(elevator.x().read(context));
                let dx = magnitude(context, difference)? as i16;
                if flag.read(context) == handle {
                    if elevator.state().read(context) != 3
                        || y <= elevator.y().read(context)
                        || dx != 0
                    {
                        return Ok(Some(1));
                    }
                    if i32::from(y) >= i32::from(elevator.y().read(context)) + 10 {
                        flag.write(context, 0);
                    }
                } else if dx < 20 {
                    if (player.y().read(context) > elevator.y().read(context)
                        && y < elevator.y().read(context))
                        || (player.y().read(context) < elevator.y().read(context)
                            && y > elevator.y().read(context))
                    {
                        return Ok(Some(1));
                    }
                    let difference = i32::from(y) - (i32::from(elevator.y().read(context)) + 5);
                    let dy = magnitude(context, difference)? as i16;
                    if dy < 7 {
                        if player.height().read(context) > 0 {
                            return Ok(Some(1));
                        }
                        if elevator.state().read(context) == 3
                            && dx < 10
                            && player.y().read(context) > elevator.y().read(context)
                        {
                            if elevation == 0
                                && i32::from(y) < i32::from(elevator.y().read(context)) + 8
                            {
                                flag.write(context, handle);
                            }
                        } else {
                            return Ok(Some(1));
                        }
                    }
                }
            }
        }
    }
    Ok(Some(0))
}

/// G/src/elevator/elevator_scene.c:285 (`ElevatorBlocksEntityMovement`).
pub fn elevator_blocks_entity_movement(
    context: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(args[0]);
    let x = args[1] as i16;
    let y = args[2] as i16;
    let _unused_index = player.entity_index().read(context);
    for index in 0..4 {
        let elevator = ram::g_elevator_records(revision)
            .cast::<ElevatorRecord>()
            .offset(index);
        if elevator.state().read(context) != 11
            && elevator.state().read(context) != 13
            && player.level().read(context) == elevator.level().read(context)
        {
            let difference = i32::from(x) - i32::from(elevator.x().read(context));
            if magnitude(context, difference)? < 20 {
                let difference = i32::from(y) - (i32::from(elevator.y().read(context)) + 5);
                let dy = magnitude(context, difference)? as i16;
                if dy < 7
                    && ((y < player.y().read(context)
                        && player.y().read(context) > elevator.y().read(context))
                        || (y > player.y().read(context)
                            && player.y().read(context) < elevator.y().read(context)))
                {
                    return Ok(Some(1));
                }
            }
        }
    }
    Ok(Some(0))
}

/// G/src/elevator/elevator_scene.c:311 (`ElevatorContainsPosition`).
/// This query deliberately does not filter absent or blocked elevator states.
pub fn elevator_contains_position(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let x = args[0] as i16;
    let y = args[1] as i16;
    let level = args[2] as i16;
    for index in 0..4 {
        let elevator = ram::g_elevator_records(revision)
            .cast::<ElevatorRecord>()
            .offset(index);
        if i16::from(elevator.level().read(context)) == level {
            let difference = i32::from(elevator.y().read(context)) - i32::from(y) + 5;
            if magnitude(context, difference)? < 15 {
                let difference = i32::from(elevator.x().read(context)) - i32::from(x);
                if magnitude(context, difference)? < 20 {
                    return Ok(Some(1));
                }
            }
        }
    }
    Ok(Some(0))
}

/// G/src/elevator/elevator_transitions.c:22 (`ElevatorCheckPlayerProximityAndInput`).
pub fn elevator_check_player_proximity_and_input(
    context: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let elevator = Address::<ElevatorRecord>::new(args[0]);
    let handle = args[1] as i8;
    let mut occupied = 0;
    for player in 0..2 {
        let entity = ram::g_player_info(revision).at(player);
        if entity.level().read(context) == elevator.level().read(context) {
            if handle
                == ram::g_player_elevator_flags(revision)
                    .at(player)
                    .read(context)
            {
                if ram::g_player_processed_input(revision)
                    .at(player)
                    .read(context)
                    & 3
                    != 0
                {
                    occupied = 1;
                }
            } else {
                let difference =
                    i32::from(entity.x().read(context)) - i32::from(elevator.x().read(context));
                if (magnitude(context, difference)? as i16) < 10 {
                    let dy = entity
                        .y()
                        .read(context)
                        .wrapping_sub(elevator.y().read(context));
                    if dy < 20
                        && ((elevator.state().read(context) <= 1 && dy >= 10)
                            || (elevator.state().read(context) == 3 && dy > 0))
                    {
                        ram::g_player_elevator_approach_input_gate(revision)
                            .at(player)
                            .write(context, 1);
                        occupied = 1;
                    }
                }
            }
        }
    }
    Ok(Some(occupied))
}

/// G/src/elevator/elevator_transitions.c:52 (`ElevatorStepDepartureHeightCycles`).
pub fn elevator_step_departure_height_cycles(
    context: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let elevator = Address::<ElevatorRecord>::new(args[0]);
    let delay = ram::g_departure_elevator_shake_delay(revision);
    let cycles = ram::g_departure_elevator_shake_cycles(revision);
    let (remaining, expired) = super::decrement_byte_and_test_le_zero(delay.read(context));
    delay.write(context, remaining);
    if expired {
        let entity =
            ram::g_player_info(revision).at(i32::from(elevator.entity().read(context)) + 2);
        let height = entity.height().read(context).wrapping_add(4);
        entity.height().write(context, height);
        if entity.height().read(context) > 28 {
            entity.height().write(context, 0);
            let remaining = cycles.read(context).wrapping_sub(1);
            cycles.write(context, remaining);
            call_void(context, ids::PLAY_SOUND, &[17, 50])?;
        }
        let remaining = cycles.read(context);
        delay.write(context, remaining / 2);
    }
    Ok(Some(u32::from(cycles.read(context) == 0)))
}
