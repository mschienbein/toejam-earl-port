use crate::{
    Context, Result, call, call_at, call_at_void,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    call_void, ids,
};
use tje_mem::{Address, ObjectState, ram};

/// G/src/elevator/elevator_transitions.c:192 (`ProcessElevatorStep2`).
/// Arrival processing deliberately selects Earl as the last same-level non-rider
/// and reloads each shared delay/flag exactly where the original does.
pub fn process_elevator_step2(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for bank in 0..2 {
        let handle = (bank * 2 + 2) as i8;
        let elevator = ram::g_elevator_records(revision).at(bank).at(1);
        let state = elevator.state().read(context);
        if state == 11 {
            continue;
        }
        let mut next = -1_i8;
        if state == 8 {
            let number = elevator.entity().read(context);
            if number != 0
                && ram::g_player_info(revision)
                    .at(i32::from(number))
                    .frame_duration()
                    .read(context)
                    == 0
            {
                next = 1;
                let mut non_rider = Address::<ObjectState>::new(0);
                for player in 0..2 {
                    let entity = ram::g_player_info(revision).at(player);
                    if handle
                        == ram::g_player_elevator_flags(revision)
                            .at(player)
                            .read(context)
                    {
                        let behavior = entity.behavior_flags().read(context) & !0x40;
                        entity.behavior_flags().write(context, behavior);
                    } else if entity.level().read(context) == elevator.level().read(context) {
                        non_rider = entity;
                    }
                }
                if non_rider.address() != 0 {
                    let difference = i32::from(non_rider.x().read(context))
                        - i32::from(elevator.x().read(context));
                    let dx = call(context, ids::ABS, &[difference as u32])? as i16;
                    let difference = i32::from(non_rider.y().read(context))
                        - (i32::from(elevator.y().read(context)) + 5);
                    let dy = call(context, ids::ABS, &[difference as u32])? as i16;
                    if dx < 22 && dy < 7 {
                        let y = elevator.y().read(context).wrapping_add(12);
                        non_rider.y().write(context, y);
                    }
                }
                call_void(context, ids::PLAY_SOUND, &[15, 50])?;
                if ram::g_current_music_selection(revision).read(context) == 6 {
                    let level = ram::g_highest_level(revision).read(context);
                    let music = call(
                        context,
                        ids::GET_LEVEL_MUSIC_INDEX,
                        &[i32::from(level) as u32],
                    )?;
                    ram::g_requested_music_selection(revision).write(context, music as i8);
                }
            }
        } else if state == 2 {
            let number = elevator.entity().read(context);
            if number != 0
                && ram::g_player_info(revision)
                    .at(i32::from(number) + 2)
                    .frame_duration()
                    .read(context)
                    == 0
            {
                next = 3;
            }
        } else if state == 4 {
            let number = elevator.entity().read(context);
            if number == 0
                || ram::g_player_info(revision)
                    .at(i32::from(number) + 2)
                    .frame_duration()
                    .read(context)
                    == 0
            {
                if handle == ram::g_player_elevator_flags(revision).at(0).read(context)
                    || handle == ram::g_player_elevator_flags(revision).at(1).read(context)
                {
                    next = 1;
                } else if elevator.entity().read(context) == 0 {
                    next = 11;
                } else {
                    next = 0;
                    ram::g_arrival_elevator_close_delay(revision).write(context, 30);
                }
            }
        } else {
            let expired = if state == 0 {
                let delay = ram::g_arrival_elevator_close_delay(revision);
                let (remaining, expired) =
                    super::decrement_byte_and_test_le_zero(delay.read(context));
                delay.write(context, remaining);
                expired
            } else {
                false
            };
            if expired {
                next = 9;
            } else if state == 9 {
                let number = elevator.entity().read(context);
                if number == 0
                    || ram::g_player_info(revision)
                        .at(i32::from(number))
                        .frame_duration()
                        .read(context)
                        == 0
                {
                    next = 11;
                }
            } else if call(
                context,
                ids::ELEVATOR_CHECK_PLAYER_PROXIMITY_AND_INPUT,
                &[elevator.address(), i32::from(handle) as u32],
            )? != 0
            {
                if state != 3 && state != 2 {
                    next = 2;
                    call_void(context, ids::PLAY_SOUND, &[24, 30])?;
                }
            } else if state > 1 && state != 4 {
                next = 4;
                call_void(context, ids::PLAY_SOUND, &[24, 30])?;
            }
        }
        for player in 0..2 {
            if handle
                == ram::g_player_elevator_flags(revision)
                    .at(player)
                    .read(context)
            {
                let entity = ram::g_player_info(revision).at(player);
                if entity.sprite_state().read(context) == 0 && entity.frame().read(context) == 1 {
                    entity.frame_duration().write(context, 0);
                }
                let x = elevator.x().read(context);
                entity.x().write(context, x);
            }
        }
        if next != -1 {
            call_void(
                context,
                ids::ELEVATOR_SET_STATE,
                &[elevator.address(), i32::from(next) as u32],
            )?;
        }
    }
    Ok(None)
}

/// G/src/elevator/elevator_transitions.c:73 (`ProcessElevatorStep1`).
/// Preserve the original MOVEM D2-D6/A2-A5 frame for the prepared long-ride edge.
pub fn process_elevator_step1(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x7c, 0x3c))?;
    let result = step1_body(context);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result.map(|()| None);
    }
    let leave = frame.leave(context);
    result?;
    leave?;
    Ok(None)
}

fn step1_body(context: &mut dyn Context) -> Result<()> {
    let revision = context.revision();
    let caller = ids::PROCESS_ELEVATOR_STEP1;
    let dispatcher = context.dispatcher();
    context.set_address_register(3, ram::g_player_info(revision).address());
    context.set_address_register(4, ram::g_player_elevator_flags(revision).address());
    for bank in 0..2 {
        let handle = (bank * 2 + 1) as i8;
        let elevator = ram::g_elevator_records(revision).at(bank).at(0);
        context.set_data_register(6, (context.data_register(6) & 0xffff0000) | bank as u32);
        context.set_data_register(2, (context.data_register(2) & 0xffffff00) | handle as u32);
        context.set_address_register(2, elevator.address());
        let state = elevator.state().read(context);
        context.set_data_register(
            3,
            (context.data_register(3) & 0xffffff00) | state as u8 as u32,
        );
        if state == 11 {
            continue;
        }
        let mut next = -1_i8;
        context.set_data_register(4, u32::MAX);
        let first = ram::g_player_info(revision).at(0);
        let second = ram::g_player_info(revision).at(1);
        let flags = ram::g_player_elevator_flags(revision);
        if state == 13 {
            let primary_level = ram::g_primary_arrival_elevator_level(revision).read(context);
            let next_level = i32::from(elevator.level().read(context)) + 1;
            if i32::from(primary_level) != next_level && {
                let secondary_level =
                    ram::g_secondary_arrival_elevator_level(revision).read(context);
                let next_level = i32::from(elevator.level().read(context)) + 1;
                i32::from(secondary_level) != next_level
            } {
                next = 10;
                let entity = if first.level().read(context) == elevator.level().read(context) {
                    first
                } else {
                    second
                };
                context.set_data_register(3, entity.address());
                context.set_address_register(5, entity.address());
                context.set_data_register(4, next as u32);
                let difference =
                    i32::from(entity.x().read(context)) - i32::from(elevator.x().read(context));
                let dx = call_at(
                    context,
                    ids::ABS,
                    &[difference as u32],
                    CallSite::relative(caller, revision, 0xa0),
                )? as i16;
                context.set_data_register(
                    5,
                    (context.data_register(5) & 0xffff0000) | dx as u16 as u32,
                );
                let difference = i32::from(entity.y().read(context))
                    - (i32::from(elevator.y().read(context)) + 5);
                let dy = call_at(
                    context,
                    ids::ABS,
                    &[difference as u32],
                    CallSite::relative(caller, revision, 0xbc),
                )? as i16;
                if dx < 22 && dy < 7 {
                    let y = elevator.y().read(context).wrapping_add(12);
                    entity.y().write(context, y);
                }
            }
        } else if state == 10 {
            if elevator.entity().read(context) != 0
                && ram::g_player_info(revision)
                    .cast::<ObjectState>()
                    .byte_offset(i32::from(
                        i16::from(elevator.entity().read(context)).wrapping_shl(7),
                    ))
                    .frame_duration()
                    .read(context)
                    == 0
            {
                if handle == flags.at(0).read(context) {
                    next = 1;
                    let behavior = first.behavior_flags().read(context) & !0x40;
                    first.behavior_flags().write(context, behavior);
                } else if handle == flags.at(1).read(context) {
                    let behavior = second.behavior_flags().read(context) & !0x40;
                    second.behavior_flags().write(context, behavior);
                    next = 1;
                } else {
                    next = 0;
                }
            }
        } else if state == 2 || state == 4 {
            if elevator.entity().read(context) != 0
                && ram::g_player_info(revision)
                    .at(i32::from(elevator.entity().read(context)) + 2)
                    .frame_duration()
                    .read(context)
                    == 0
            {
                if state == 2 {
                    next = 3;
                } else if handle == flags.at(0).read(context) || handle == flags.at(1).read(context)
                {
                    next = 1;
                } else {
                    next = 0;
                }
            }
        } else if state == 6 {
            if elevator.entity().read(context) == 0 {
                continue;
            }
            let entity =
                ram::g_player_info(revision).at(i32::from(elevator.entity().read(context)) + 2);
            let frame = i16::from(entity.frame().read(context));
            context.set_data_register(
                5,
                (context.data_register(5) & 0xffff0000) | frame as u16 as u32,
            );
            let entity =
                ram::g_player_info(revision).at(i32::from(elevator.entity().read(context)) + 2);
            if entity.frame_duration().read(context) == 0 {
                next = 5;
                ram::g_departure_elevator_shake_cycles(revision).write(context, 5);
                ram::g_departure_elevator_shake_delay(revision).write(context, 2);
                context.set_data_register(4, next as u32);
                let sound_arguments = OutgoingArguments::push(context, &[17, 50])?;
                sound_arguments.call_void(
                    context,
                    dispatcher,
                    ids::PLAY_SOUND,
                    CallSite::relative(caller, revision, 0x1d6),
                )?;
                let first_life = ram::g_player_lives(revision).at(0).read(context);
                sound_arguments.release(context)?;
                if (first_life < 0 || handle == flags.at(0).read(context))
                    && (ram::g_player_lives(revision).at(1).read(context) < 0
                        || handle == flags.at(1).read(context))
                {
                    call_at_void(
                        context,
                        ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
                        &[],
                        CallSite::relative(caller, revision, 0x1fc),
                    )?;
                    call_at_void(
                        context,
                        ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
                        &[],
                        CallSite::relative(caller, revision, 0x202),
                    )?;
                }
            } else if frame < 12 && ((i32::from(frame) + 1) & 3) == 0 {
                call_at_void(
                    context,
                    ids::PLAY_SOUND,
                    &[7, 50],
                    CallSite::relative(caller, revision, 0x352),
                )?;
            }
        } else if state == 5 {
            if elevator.entity().read(context) != 0
                && call_at(
                    context,
                    ids::ELEVATOR_STEP_DEPARTURE_HEIGHT_CYCLES,
                    &[elevator.address()],
                    CallSite::relative(caller, revision, 0x23c),
                )? != 0
            {
                next = 7;
                context.set_data_register(4, next as u32);
                call_at_void(
                    context,
                    ids::PLAY_SOUND,
                    &[3, 65],
                    CallSite::relative(caller, revision, 0x352),
                )?;
            }
        } else if state == 7 {
            if elevator.entity().read(context) != 0
                && ram::g_player_info(revision)
                    .cast::<ObjectState>()
                    .byte_offset(i32::from(
                        i16::from(elevator.entity().read(context)).wrapping_shl(7),
                    ))
                    .frame_duration()
                    .read(context)
                    == 0
            {
                if second.level().read(context) == first.level().read(context)
                    && flags.at(1).read(context) != flags.at(0).read(context)
                {
                    next = 10;
                } else {
                    next = 11;
                    context.set_data_register(4, next as u32);
                    call_at_void(
                        context,
                        ids::PLAY_SOUND,
                        &[0, 98],
                        CallSite::relative(caller, revision, 0x352),
                    )?;
                }
            }
        } else if (handle == flags.at(0).read(context)
            && handle == flags.at(1).read(context)
            && ram::g_player_inventory_open(revision).at(0).read(context) == 0
            && ram::g_player_inventory_open(revision).at(1).read(context) == 0)
            || (handle == flags.at(0).read(context)
                && ram::g_player_inventory_open(revision).at(0).read(context) == 0
                && second.level().read(context) > first.level().read(context))
            || (handle == flags.at(1).read(context)
                && ram::g_player_inventory_open(revision).at(1).read(context) == 0
                && first.level().read(context) > second.level().read(context))
        {
            if state == 3 {
                next = 4;
            } else if state == 1 {
                ram::g_elevator_departure_in_progress(revision).write(context, 1);
                next = 6;
                if handle == flags.at(0).read(context) {
                    let behavior = first.behavior_flags().read(context) | 0x40;
                    first.behavior_flags().write(context, behavior);
                }
                if handle == flags.at(1).read(context) {
                    let behavior = second.behavior_flags().read(context) | 0x40;
                    second.behavior_flags().write(context, behavior);
                }
            }
        } else if call_at(
            context,
            ids::ELEVATOR_CHECK_PLAYER_PROXIMITY_AND_INPUT,
            &[elevator.address(), i32::from(handle) as u32],
            CallSite::relative(caller, revision, 0x320),
        )? != 0
        {
            if state != 3 && state != 2 {
                next = 2;
                context.set_data_register(4, next as u32);
                call_at_void(
                    context,
                    ids::PLAY_SOUND,
                    &[24, 30],
                    CallSite::relative(caller, revision, 0x352),
                )?;
            }
        } else if state > 1 && state != 4 {
            next = 4;
            context.set_data_register(4, next as u32);
            call_at_void(
                context,
                ids::PLAY_SOUND,
                &[24, 30],
                CallSite::relative(caller, revision, 0x352),
            )?;
        }
        for player in 0..2 {
            if handle == flags.at(player).read(context) {
                let entity = ram::g_player_info(revision).at(player);
                // The original reuses D3 for the last attached player's pointer;
                // AdvanceRiders observes that residue on its no-rider path.
                context.set_data_register(3, entity.address());
                if entity.sprite_state().read(context) == 0 && entity.frame().read(context) == 1 {
                    context.discard_u8(entity.frame_duration().address())?;
                    entity.frame_duration().write(context, 0);
                }
                let x = elevator.x().read(context);
                entity.x().write(context, x);
            }
        }
        context.set_data_register(
            4,
            if next == 0 {
                context.data_register(4) & 0xffffff00
            } else {
                i32::from(next) as u32
            },
        );
        if next != -1 {
            call_at_void(
                context,
                ids::ELEVATOR_SET_STATE,
                &[elevator.address(), i32::from(next) as u32],
                CallSite::relative(caller, revision, 0x3a4),
            )?;
        }
        if next == 11 {
            call_at_void(
                context,
                ids::ELEVATOR_ADVANCE_RIDERS_TO_NEXT_LEVEL,
                &[i32::from(handle) as u32],
                CallSite::relative(ids::PROCESS_ELEVATOR_STEP1, revision, 0x3ba),
            )?;
        }
    }
    Ok(())
}

/// G/src/elevator/elevator_transitions.c:270 (`ProcessElevators`).
pub fn process_elevators(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    call_at_void(
        context,
        ids::PROCESS_ELEVATOR_STEP1,
        &[],
        CallSite::relative(ids::PROCESS_ELEVATORS, revision, 4),
    )?;
    call_at_void(
        context,
        ids::PROCESS_ELEVATOR_STEP2,
        &[],
        CallSite::relative(ids::PROCESS_ELEVATORS, revision, 8),
    )?;
    Ok(None)
}
