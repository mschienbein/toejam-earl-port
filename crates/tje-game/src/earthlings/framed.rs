use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, FunctionId, Result, call_at, ids};
use tje_mem::{Address, ObjectState, Revision, ram, rom};

// Return PCs are inventory executable-call sites plus the matching compiler's
// JSR width: register=2, PC-relative=4, absolute-long=6. REV 00 offsets agree.
fn invoke(
    context: &mut dyn Context,
    caller: FunctionId,
    callee: FunctionId,
    args: &[u32],
    return_rev02: u32,
) -> Result<u32> {
    let offset = return_rev02 - caller.metadata().address(Revision::Rev02);
    let site = CallSite::relative(caller, context.revision(), offset);
    call_at(context, callee, args, site)
}
pub(super) fn finish(
    context: &mut dyn Context,
    frame: LocalFrame,
    value: Result<Option<u32>>,
) -> Result<Option<u32>> {
    if value
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return value;
    }
    let leave = frame.leave(context);
    match (value, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

/// G/src/earthlings/spawning_and_banks.c:101 (`FindSpawnLocationForEarthling`).
/// Atomic; exact LINK #-16 / MOVEM D2-D7,A2-A4 frame. TerrainValidation receives
/// real -16/-14(A6) output words. No Rust stack pointer or invented RAM scratch.
pub fn find_spawn_location_for_earthling(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(16, 0xfc, 0x1c))?;
    let result = find_spawn(context, arguments, &frame);
    finish(context, frame, result)
}
fn find_spawn(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::FIND_SPAWN_LOCATION_FOR_EARTHLING;
    let kind = arguments[0] as i16;
    let bank = arguments[1] as i16;
    let level = arguments[4] as i16;
    let map = if i16::from(ram::g_player1_map_level(revision).read(context)) == level {
        ram::g_player1_level_map(revision).cast::<u16>()
    } else {
        ram::g_player2_level_map(revision).cast::<u16>()
    };
    // c_spawning_and_banks.s:196–203: MULU #38 then ADDA.W, not long scaling.
    let definition = rom::earthling_definitions(revision)
        .at(0)
        .byte_offset(i32::from(kind.wrapping_mul(38)));
    frame.local::<u32>(-12).write(context, definition.address());
    let (width, height, mask) = if [0, 12, 19, 13, 18].contains(&kind) {
        (16_i16, 8_i16, 3_u8)
    } else if kind == 8 {
        (2, 2, 2)
    } else if kind == 14 {
        (3, 2, 8)
    } else {
        (8, 2, 3)
    };
    frame.local::<i16>(-2).write(context, height);
    frame.local::<u8>(-5).write(context, mask);
    let x = frame.local::<i16>(-16);
    let y = frame.local::<i16>(-14);
    for _ in 0..10 {
        let row =
            (invoke(context, caller, ids::GET_RANDOM_NUMBER, &[], 0x017868)? as u16 % 26) as i16;
        let column =
            (invoke(context, caller, ids::GET_RANDOM_NUMBER, &[], 0x017884)? as u16 % 19) as i16;
        if spawn_probe(
            context,
            frame,
            map,
            width,
            (bank, level),
            (row, column),
            (0x0178ec, 0x017936),
        )? {
            Address::<i16>::new(arguments[2]).write(context, x.read(context));
            Address::<i16>::new(arguments[3]).write(context, y.read(context));
            return Ok(Some(0));
        }
    }
    let start_row =
        (invoke(context, caller, ids::GET_RANDOM_NUMBER, &[], 0x01795c)? as u16 % 26) as i16;
    let start_column =
        (invoke(context, caller, ids::GET_RANDOM_NUMBER, &[], 0x017978)? as u16 % 19) as i16;
    frame.local::<i16>(-4).write(context, start_column);
    let mut row = start_row;
    loop {
        let mut column = start_column;
        loop {
            if spawn_probe(
                context,
                frame,
                map,
                width,
                (bank, level),
                (row, column),
                (0x0179e8, 0x017a32),
            )? {
                Address::<i16>::new(arguments[2]).write(context, x.read(context));
                Address::<i16>::new(arguments[3]).write(context, y.read(context));
                return Ok(Some(0));
            }
            column = (column + 1) % 19;
            if column == start_column {
                break;
            }
        }
        row = (row + 1) % 26;
        if row == start_row {
            break;
        }
    }
    Ok(Some(1))
}
fn spawn_probe(
    context: &mut dyn Context,
    frame: &LocalFrame,
    map: Address<u16>,
    width: i16,
    bank_level: (i16, i16),
    position: (i16, i16),
    sites: (u32, u32),
) -> Result<bool> {
    let (bank, level) = bank_level;
    let (row, column) = position;
    let (terrain_pc, overlap_pc) = sites;
    let caller = ids::FIND_SPAWN_LOCATION_FOR_EARTHLING;
    let tile = u32::from(
        map.offset(i32::from(row) * 19 + i32::from(column))
            .read(context),
    );
    let x = frame.local::<i16>(-16);
    let y = frame.local::<i16>(-14);
    let height = frame.local::<i16>(-2).read(context) as u32;
    let mask = u32::from(frame.local::<u8>(-5).read(context));
    if invoke(
        context,
        caller,
        ids::TERRAIN_VALIDATION,
        &[tile, width as u32, height, x.address(), y.address(), mask],
        terrain_pc,
    )? == 0
    {
        return Ok(false);
    }
    y.write(context, y.read(context).wrapping_add((row + 1) * 64));
    x.write(context, x.read(context).wrapping_add((column + 1) * 128));
    let definition = frame.local::<u32>(-12).read(context);
    let px = x.read(context) as u32;
    let py = y.read(context) as u32;
    Ok(invoke(
        context,
        caller,
        ids::CHECK_EARTHLING_OVERLAP,
        &[definition, bank as u32, px, py, level as u32],
        overlap_pc,
    )? != 0)
}

/// G/src/earthlings/spawning_and_banks.c:190 (`EarthlingRetainLevelBank`).
/// Atomic; LINK #-8 / MOVEM D2-D6,A2-A4. Coordinates and the special/bank slots
/// keep their emitted addresses and retain residual bytes until an original store.
pub fn earthling_retain_level_bank(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0x7c, 0x1c))?;
    let result = retain(context, arguments[0] as i16, &frame);
    finish(context, frame, result)
}
fn retain(context: &mut dyn Context, level: i16, frame: &LocalFrame) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::EARTHLING_RETAIN_LEVEL_BANK;
    if level >= 26 {
        return Ok(None);
    }
    let bank_slot = frame.local::<i16>(-4);
    for bank in 0..2 {
        bank_slot.write(context, bank as i16);
        let users = ram::g_earthling_bank_users(revision).at(bank);
        if users.read(context) != 0
            && i16::from(
                ram::g_earthling_bank_levels(revision)
                    .at(bank)
                    .read(context),
            ) == level
        {
            users.write(context, users.read(context).wrapping_add(1));
            return Ok(None);
        }
    }
    let bank = i32::from(ram::g_earthling_bank_users(revision).at(0).read(context) != 0);
    bank_slot.write(context, bank as i16);
    // c_spawning_and_banks.s:547–551: word-scaled seed subscript.
    let seed = ram::g_level_seeds(revision)
        .at(0)
        .byte_offset(i32::from(level.wrapping_mul(2)))
        .read(context);
    let mut saved = invoke(
        context,
        caller,
        ids::STACK_BASED_RANDOM,
        &[u32::from(seed)],
        0x017b52,
    )?;
    let mut ship = false;
    if level != 0 {
        for slot in 0..10 {
            if i16::from(
                ram::g_ship_piece_level_assignments(revision)
                    .at(slot)
                    .read(context),
            ) == level
            {
                ship = true
            }
        }
    }
    let special = invoke(
        context,
        caller,
        ids::EARTHLING_CONFIGURE_MAILBOX_LEVEL_BANK,
        &[bank as u32, level as u32],
        0x017b8c,
    )? as i8;
    frame.local::<i8>(-1).write(context, special);
    let x = frame.local::<i16>(-8);
    let y = frame.local::<i16>(-6);
    for slot in 0..20 {
        let kind;
        if level == 1 {
            if saved != 0 {
                invoke(context, caller, ids::STACK_BASED_RANDOM, &[saved], 0x017ba6)?;
                saved = 0;
            }
            if slot == 0 {
                break;
            }
            kind = 18;
        } else if frame.local::<i8>(-1).read(context) != 0 {
            kind = 9;
            frame.local::<i8>(-1).write(context, 0);
        } else if ship {
            kind = 0;
            invoke(
                context,
                caller,
                ids::MAP_GET_SHIP_PIECE_WORLD_POSITION,
                &[x.address(), y.address()],
                0x017bda,
            )?;
            ship = false;
            if i16::from(ram::g_highest_level(revision).read(context)) == level {
                ram::g_ship_screen_state(revision)
                    .pending_piece_notice_level()
                    .write(context, level as i8);
            }
        } else {
            if saved != 0 {
                invoke(context, caller, ids::STACK_BASED_RANDOM, &[saved], 0x017bfe)?;
                saved = 0;
            }
            // c_spawning_and_banks.s:627–635: level*20 uses word arithmetic.
            let choices = rom::level_earthling_choices(revision)
                .cast::<i8>()
                .byte_offset(i32::from(level.wrapping_mul(20)));
            let choice = choices.offset(slot).read(context);
            kind = if choice == 20
                && i16::from(ram::g_highest_level(revision).read(context)) > level
            {
                1
            } else {
                choice
            };
        }
        let mask = ram::g_earthling_spawn_failure_masks(revision)
            .at(0)
            .byte_offset(i32::from(level.wrapping_mul(4)));
        if kind > 0 {
            if level == 0 {
                invoke(
                    context,
                    caller,
                    ids::GET_LEVEL_ZERO_EARTHLING_POSITION,
                    &[kind as u32, x.address(), y.address()],
                    0x017c4e,
                )?;
            } else if invoke(
                context,
                caller,
                ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
                &[
                    kind as u32,
                    bank as u32,
                    x.address(),
                    y.address(),
                    level as u32,
                ],
                0x017c76,
            )? != 0
            {
                mask.write(context, mask.read(context) | (0x8000_0000_u32 >> slot));
            }
        }
        let record = ram::g_earthling_banks(revision).at(bank).at(slot);
        if kind == 0 || (kind >= 0 && mask.read(context) & (0x8000_0000_u32 >> slot) == 0) {
            record.kind().write(context, kind);
            record.level().write(context, level as i8);
            record.x().write(context, x.read(context));
            record.y().write(context, y.read(context));
            record.saved_y().write(context, y.read(context));
            if kind == 19 || kind == 9 {
                record.saved_x_or_status().write(context, 0);
                record.saved_y().write(context, 0);
            } else if kind == 7 {
                record.saved_x_or_status().write(context, 1);
            } else {
                record.saved_x_or_status().write(context, x.read(context));
            }
            let definition = rom::earthling_definitions(revision).at(i32::from(kind));
            record
                .health_visibility()
                .at(0)
                .write(context, definition.health().read(context));
            record.health_visibility().at(1).write(context, 0);
        } else {
            record.kind().write(context, -1)
        }
    }
    let users = ram::g_earthling_bank_users(revision).at(bank);
    users.write(context, users.read(context).wrapping_add(1));
    ram::g_earthling_bank_levels(revision)
        .at(bank)
        .write(context, level as i8);
    Ok(None)
}

/// G/src/earthlings/targeting.c:18 (`EarthlingSteerTowardNearestTarget`).
/// Atomic with LINK #-12 / MOVEM D2-D6,A2-A4. The conditionally initialized
/// delta words and entry A3 target residue are deliberately not replaced by zero.
pub fn earthling_steer_toward_nearest_target(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let mut target = Address::<ObjectState>::new(context.address_register(3));
    let frame = LocalFrame::enter(context, FrameLayout::link(12, 0x7c, 0x1c))?;
    let result = steer(context, arguments, &frame, &mut target);
    finish(context, frame, result)
}
fn candidate(
    context: &mut dyn Context,
    frame: &LocalFrame,
    state: Address<ObjectState>,
    target: Address<ObjectState>,
    player: i32,
    separation: i16,
    abs_pcs: (u32, u32),
) -> Result<()> {
    let (abs_x_pc, abs_y_pc) = abs_pcs;
    let dx = frame.local::<i16>(-8 + player * 2);
    let dy = frame.local::<i16>(-4 + player * 2);
    dx.write(
        context,
        ((i32::from(target.x().read(context)) - i32::from(state.x().read(context))) >> 2) as i16,
    );
    let bias = if state.x().read(context) < target.x().read(context) {
        dx.read(context).wrapping_sub(separation)
    } else {
        dx.read(context).wrapping_add(separation)
    };
    dx.write(context, bias);
    dy.write(
        context,
        ((i32::from(target.y().read(context)) - i32::from(state.y().read(context))) >> 2) as i16,
    );
    let x = dx.read(context) as u32;
    if invoke(
        context,
        ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
        ids::ABS,
        &[x],
        abs_x_pc,
    )? as i32
        >= 160
    {
        return Ok(());
    }
    let y = dy.read(context) as u32;
    if (invoke(
        context,
        ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
        ids::ABS,
        &[y],
        abs_y_pc,
    )? as i32)
        < 112
    {
        let x = i32::from(dx.read(context));
        let y = i32::from(dy.read(context));
        frame.local::<u16>(-12 + player * 2).write(
            context,
            (x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y))) as u16,
        );
    }
    Ok(())
}
fn steer(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
    target: &mut Address<ObjectState>,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET;
    let state = Address::<ObjectState>::new(arguments[0]);
    let mode = arguments[3] as i8;
    let separation = (arguments[2] as i16) >> 2;
    let mut maximum = if state.sprite_state().read(context) == 2 {
        0x400
    } else {
        (arguments[1] as u16).wrapping_shl(8)
    };
    let d0 = frame.local::<u16>(-12);
    let d1 = frame.local::<u16>(-10);
    d0.write(context, 0x7fff);
    d1.write(context, 0x7fff);
    for player in 0..2 {
        let decoy = ram::g_player_decoy_entities(revision)
            .at(player)
            .read(context);
        if decoy == 0 {
            continue;
        }
        *target = ram::g_player_info(revision).at(i32::from(decoy));
        if state.level().read(context) == target.level().read(context) {
            let (xpc, ypc) = if player == 0 {
                (0x01aad4, 0x01aae8)
            } else {
                (0x01ab8a, 0x01ab9e)
            };
            candidate(
                context,
                frame,
                state,
                *target,
                player,
                separation,
                (xpc, ypc),
            )?;
        }
        let distance = frame.local::<u16>(-12 + player * 2);
        if distance.read(context) > 0x1800 {
            distance.write(context, 0x7fff)
        } else {
            maximum = distance.read(context)
        }
    }
    let special = d0.read(context) != 0x7fff || d1.read(context) != 0x7fff;
    if !special {
        for player in 0..2 {
            let player_state = ram::g_player_info(revision).at(player);
            if state.level().read(context) != player_state.level().read(context)
                || ram::g_player_elevator_flags(revision)
                    .at(player)
                    .read(context)
                    != 0
                || (mode == 7
                    && ram::g_player_control_remap_entities(revision)
                        .at(player)
                        .read(context)
                        != 0)
            {
                continue;
            }
            *target = player_state;
            let latch = if player == 0 {
                state.attachment_x_or_latch()
            } else {
                state.attachment_y_or_latch()
            };
            if mode == 18
                && (ram::g_player_inventory(revision)
                    .at(player)
                    .at(0)
                    .read(context)
                    == -1
                    || latch.read(context) != 0)
            {
                continue;
            }
            let eligible_pc = if player == 0 { 0x01ac40 } else { 0x01ad18 };
            if invoke(
                context,
                caller,
                ids::EARTHLING_CAN_TARGET_PLAYER_STATE,
                &[state.address(), target.address()],
                eligible_pc,
            )? != 0
            {
                let (xpc, ypc) = if player == 0 {
                    (0x01ac8c, 0x01aca0)
                } else {
                    (0x01ad64, 0x01ad78)
                };
                candidate(
                    context,
                    frame,
                    state,
                    *target,
                    player,
                    separation,
                    (xpc, ypc),
                )?;
            }
        }
    }
    if maximum < d0.read(context) && maximum < d1.read(context) {
        return Ok(Some(u32::MAX));
    }
    let selected = if d0.read(context) < d1.read(context) {
        0
    } else {
        1
    };
    let result = if mode == 12 {
        let (dx, dy) = selected_deltas(context, frame, selected);
        invoke(
            context,
            caller,
            ids::ENTITY_STEER_WITH_VERTICAL_DEAD_ZONE,
            &[state.address(), dx, dy],
            0x01adea,
        )?
    } else if mode == 7 {
        let target = if special {
            ram::g_player_info(revision).at(i32::from(
                ram::g_player_decoy_entities(revision)
                    .at(selected)
                    .read(context),
            ))
        } else {
            ram::g_player_info(revision).at(selected)
        };
        let (dx, dy) = selected_deltas(context, frame, selected);
        invoke(
            context,
            caller,
            ids::ENTITY_STEER_WITH_HORIZONTAL_DRIFT,
            &[state.address(), target.address(), dx, dy],
            0x01ae3a,
        )?
    } else {
        if mode == 14 && !special {
            invoke(
                context,
                caller,
                ids::ENTITY_TRIGGER_TARGET_STATUS_EVENT_ONCE,
                &[state.address(), target.address()],
                0x01ae56,
            )?;
        }
        let (dx, dy) = selected_deltas(context, frame, selected);
        invoke(
            context,
            caller,
            ids::ENTITY_SET_VELOCITY_TOWARD_DELTA,
            &[state.address(), dx, dy],
            0x01ae7c,
        )?
    };
    Ok(Some(result as i8 as u32))
}

fn selected_deltas(context: &dyn Context, frame: &LocalFrame, player: i32) -> (u32, u32) {
    // MCC68K pushes right to left: Y is consumed before X, including residuals.
    let dy = frame.local::<i16>(-4 + player * 2).read(context) as u32;
    let dx = frame.local::<i16>(-8 + player * 2).read(context) as u32;
    (dx, dy)
}
