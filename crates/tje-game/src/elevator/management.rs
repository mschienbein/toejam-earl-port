use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ElevatorRecord, ram};

/// G/src/elevator/elevator_scene.c:97 (`SetupElevator`).
/// The invalid-level ORIGINAL BUG returns the sign-extended low byte left by
/// the emitted load of gPlayerInfo + 128, not an invented zero or error.
pub fn setup_elevator(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let level = args[0] as i8;
    let kind = args[1] as i8;
    if !(1..25).contains(&level) && kind == 0 {
        return Ok(Some(
            i32::from(ram::g_player_info(revision).at(1).address() as i8) as u32,
        ));
    }
    let mut handle = kind.wrapping_add(1);
    let mut elevator = ram::g_elevator_records(revision).at(0).at(i32::from(kind));
    let alternate = ram::g_elevator_records(revision).at(1).at(i32::from(kind));
    if level == elevator.level().read(context) {
        return Ok(Some(i32::from(handle) as u32));
    }
    if level == alternate.level().read(context) {
        return Ok(Some(i32::from(kind.wrapping_add(3)) as u32));
    }
    let first = ram::g_player_info(revision).at(0);
    let second = ram::g_player_info(revision).at(1);
    if first.level().read(context) != second.level().read(context)
        && ((level == first.level().read(context)
            && elevator.level().read(context) == second.level().read(context))
            || (level == second.level().read(context)
                && elevator.level().read(context) == first.level().read(context)))
    {
        elevator = alternate;
        handle = kind.wrapping_add(3);
    }
    elevator.level().write(context, level);
    if elevator.entity().read(context) != 0 {
        let number = i32::from(elevator.entity().read(context));
        call_void(context, ids::FREE_ENTITY, &[number as u32])?;
        let number = i32::from(elevator.entity().read(context)) + 2;
        call_void(context, ids::FREE_ENTITY, &[number as u32])?;
        elevator.entity().write(context, 0);
    }
    if kind == 0 {
        let next_level = i32::from(level) + 1;
        let blocked = i32::from(
            ram::g_elevator_records(revision)
                .at(0)
                .at(1)
                .level()
                .read(context),
        ) == next_level
            || i32::from(
                ram::g_elevator_records(revision)
                    .at(1)
                    .at(1)
                    .level()
                    .read(context),
            ) == next_level;
        elevator
            .state()
            .write(context, if blocked { 13 } else { 0 });
    } else {
        elevator.state().write(context, 8);
    }
    let (map_x, map_y) = if level == 1 {
        (9, 10)
    } else if level == 26 {
        (2, 21)
    } else {
        let position = ram::g_level_anchor_positions(revision).at(i32::from(kind));
        (position.x().read(context), position.y().read(context))
    };
    elevator
        .x()
        .write(context, ((i32::from(map_x) + 2) * 128 - 16) as i16);
    elevator
        .y()
        .write(context, ((i32::from(map_y) + 2) * 64 - 5) as i16);
    Ok(Some(i32::from(handle) as u32))
}

/// G/src/elevator/elevator_scene.c:149 (`SetupInitialElevator`).
pub fn setup_initial_elevator(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    call(context, ids::SETUP_ELEVATOR, &[1, 0])?;
    for (bank, kind) in [(0, 1), (1, 0), (1, 1)] {
        let record = ram::g_elevator_records(revision).at(bank).at(kind);
        record.state().write(context, 11);
        record.level().write(context, -1);
    }
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:161 (`ElevatorCreateEntityPair`).
pub fn elevator_create_entity_pair(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let elevator = Address::<ElevatorRecord>::new(args[0]);
    let mut number = 2;
    if ram::g_player_info(revision)
        .at(2)
        .draw_flags()
        .read(context)
        & 1
        != 0
    {
        number = 3;
        if ram::g_player_info(revision)
            .at(3)
            .draw_flags()
            .read(context)
            & 1
            != 0
        {
            return Ok(None);
        }
    }
    elevator.entity().write(context, number as i8);
    for offset in [0, 2] {
        let entity = ram::g_player_info(revision).at(number + offset);
        call_void(
            context,
            ids::RESET_ENTITY_RUNTIME_STATE,
            &[entity.address()],
        )?;
        let x = elevator.x().read(context);
        entity.x().write(context, x);
        let y = elevator.y().read(context);
        entity.y().write(context, y);
        entity.height().write(context, 0);
        let level = elevator.level().read(context);
        entity.level().write(context, level);
        entity.width().write(context, 40);
        entity.height_pixels().write(context, 48);
        entity.sprite_count().write(context, 2);
        entity.tile_count().write(context, 28);
        entity.tile().write(context, 0x8000);
        entity.draw_flags().write(context, 1);
        let behavior = entity.behavior_flags().read(context) | 1;
        entity.behavior_flags().write(context, behavior);
    }
    let state = i32::from(elevator.state().read(context)) as u32;
    call_void(
        context,
        ids::ELEVATOR_SET_STATE,
        &[elevator.address(), state],
    )?;
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:204 (`ElevatorUpdateEntityVisibility`).
pub fn elevator_update_entity_visibility(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for bank in 0..2 {
        for kind in 0..2 {
            let elevator = ram::g_elevator_records(revision).at(bank).at(kind);
            let primary_x = elevator
                .x()
                .read(context)
                .wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
            let primary_y = elevator
                .y()
                .read(context)
                .wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
            let secondary_x = elevator
                .x()
                .read(context)
                .wrapping_sub(ram::g_camera2_pending_x(revision).read(context));
            let secondary_y = elevator
                .y()
                .read(context)
                .wrapping_sub(ram::g_camera2_pending_y(revision).read(context));
            if elevator.state().read(context) != 11
                && elevator.state().read(context) != 13
                && elevator.level().read(context) > 0
                && ((elevator.level().read(context)
                    == ram::g_player1_map_level(revision).read(context)
                    && i32::from(primary_x) + 160 > 0
                    && primary_x < 480
                    && i32::from(primary_y) + 112 > 0
                    && primary_y < 336)
                    || (elevator.level().read(context)
                        == ram::g_player2_map_level(revision).read(context)
                        && ram::g_split_screen_enabled(revision).read(context) != 0
                        && i32::from(secondary_x) + 160 > 0
                        && secondary_x < 480
                        && i32::from(secondary_y) + 112 > 0
                        && secondary_y < 336))
            {
                if elevator.entity().read(context) == 0 {
                    call_void(
                        context,
                        ids::ELEVATOR_CREATE_ENTITY_PAIR,
                        &[elevator.address()],
                    )?;
                }
            } else {
                if elevator.entity().read(context) != 0 {
                    let number = i32::from(elevator.entity().read(context));
                    call_void(context, ids::FREE_ENTITY, &[number as u32])?;
                    let number = i32::from(elevator.entity().read(context)) + 2;
                    call_void(context, ids::FREE_ENTITY, &[number as u32])?;
                    elevator.entity().write(context, 0);
                }
                if elevator.state().read(context) == 11 {
                    elevator.level().write(context, -1);
                }
            }
        }
    }
    Ok(None)
}
