use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call_at, ids};
use tje_mem::{Address, LevelItem, Ptr, ram, rom};

/// G/src/presents/level_item_setup.c:152 (`MapFindItemPlacementPosition`).
/// Atomic: terrain/RNG search, with exact original local coordinate addresses.
/// Both matching objects use LINK -16; MOVEM D2-D7/A2-A5. Return-PC offsets
/// below are from c_level_item_setup.o function offset 0x302, in both revisions.
pub fn map_find_item_placement_position(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(16, 0xfc, 0x3c))?;
    let result = placement_body(context, arguments, &frame);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn placement_body(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::MAP_FIND_ITEM_PLACEMENT_POSITION;
    let output_x = Address::<i16>::new(arguments[0]);
    let output_y = Address::<i16>::new(arguments[1]);
    let level = arguments[2] as i16;
    let map = frame.local::<Ptr<u16>>(-12);
    let selected = if i16::from(ram::g_player1_map_level(revision).read(context)) == level {
        ram::g_player1_level_map(revision).cast::<u16>()
    } else {
        ram::g_player2_level_map(revision).cast::<u16>()
    };
    map.write(context, Ptr::new(selected.address()));
    let width = frame.local::<i16>(-4);
    let height = frame.local::<i16>(-6);
    if frame.argument_word(3).read(context) != 0 {
        width.write(context, 4);
        height.write(context, 2);
    } else {
        width.write(context, 3);
        height.write(context, 1);
    }
    let x = frame.local::<i16>(-16);
    let y = frame.local::<i16>(-14);
    let mut attempts = 0_i16;
    while attempts < 10 {
        let row = (call_at(
            context,
            ids::GET_RANDOM_NUMBER,
            &[],
            CallSite::relative(caller, revision, 0x58),
        )? as u16
            % 26) as i16;
        let column = (call_at(
            context,
            ids::GET_RANDOM_NUMBER,
            &[],
            CallSite::relative(caller, revision, 0x74),
        )? as u16
            % 19) as i16;
        let metatile = map
            .read(context)
            .target()
            .offset(i32::from(row) * 19 + i32::from(column))
            .read(context);
        let excluded = if metatile & 0x2000 != 0 {
            true
        } else if frame.argument_word(3).read(context) != 0 {
            call_at(
                context,
                ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
                &[u32::from(metatile)],
                CallSite::relative(caller, revision, 0xd8),
            )? != 0
        } else {
            false
        };
        if excluded {
            // The original does not consume an attempt for a marked tile.
            attempts = attempts.wrapping_sub(1);
        } else {
            let args = [
                u32::from(metatile),
                width.read(context) as u32,
                height.read(context) as u32,
                x.address(),
                y.address(),
                3,
            ];
            if call_at(
                context,
                ids::TERRAIN_VALIDATION,
                &args,
                CallSite::relative(caller, revision, 0x10c),
            )? != 0
            {
                y.write(
                    context,
                    y.read(context)
                        .wrapping_add(((i32::from(row) + 1) * 64) as i16),
                );
                x.write(
                    context,
                    x.read(context)
                        .wrapping_add(((i32::from(column) + 1) * 128) as i16),
                );
                let args = [x.read(context) as u32, y.read(context) as u32, level as u32];
                if call_at(
                    context,
                    ids::ELEVATOR_ALLOWS_WORLD_PLACEMENT,
                    &args,
                    CallSite::relative(caller, revision, 0x14a),
                )? != 0
                {
                    output_x.write(context, x.read(context));
                    output_y.write(context, y.read(context));
                    return Ok(None);
                }
            }
        }
        attempts = attempts.wrapping_add(1);
    }
    let start_row = (call_at(
        context,
        ids::GET_RANDOM_NUMBER,
        &[],
        CallSite::relative(caller, revision, 0x16a),
    )? as u16
        % 26) as i16;
    let start_column = frame.local::<i16>(-2);
    let column = (call_at(
        context,
        ids::GET_RANDOM_NUMBER,
        &[],
        CallSite::relative(caller, revision, 0x186),
    )? as u16
        % 19) as i16;
    start_column.write(context, column);
    let mut row = start_row;
    loop {
        let mut column = start_column.read(context);
        loop {
            let metatile = map
                .read(context)
                .target()
                .offset(i32::from(row) * 19 + i32::from(column))
                .read(context);
            let excluded = if metatile & 0x2000 != 0 {
                true
            } else if frame.argument_word(3).read(context) != 0 {
                call_at(
                    context,
                    ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
                    &[u32::from(metatile)],
                    CallSite::relative(caller, revision, 0x1f2),
                )? != 0
            } else {
                false
            };
            if !excluded {
                let args = [
                    u32::from(metatile),
                    width.read(context) as u32,
                    height.read(context) as u32,
                    x.address(),
                    y.address(),
                    3,
                ];
                if call_at(
                    context,
                    ids::TERRAIN_VALIDATION,
                    &args,
                    CallSite::relative(caller, revision, 0x23c),
                )? != 0
                {
                    y.write(
                        context,
                        y.read(context)
                            .wrapping_add(((i32::from(row) + 1) * 64) as i16),
                    );
                    x.write(
                        context,
                        x.read(context)
                            .wrapping_add(((i32::from(column) + 1) * 128) as i16),
                    );
                    let args = [x.read(context) as u32, y.read(context) as u32, level as u32];
                    if call_at(
                        context,
                        ids::ELEVATOR_ALLOWS_WORLD_PLACEMENT,
                        &args,
                        CallSite::relative(caller, revision, 0x27a),
                    )? != 0
                    {
                        output_x.write(context, x.read(context));
                        output_y.write(context, y.read(context));
                        return Ok(None);
                    }
                }
            }
            column = ((i32::from(column) + 1) % 19) as i16;
            if column == start_column.read(context) {
                break;
            }
        }
        row = ((i32::from(row) + 1) % 26) as i16;
        if row == start_row {
            break;
        }
    }
    Ok(None)
}

/// G/src/presents/level_item_setup.c:225 (`MapPlaceLargeLevelItems`).
/// Atomic: source-backed LINK -12; MOVEM D2-D6/A2-A3, with persistent
/// position locals across calls. Call-PC offsets derive from object start 0x5d8.
pub fn map_place_large_level_items(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(12, 0x7c, 0x0c))?;
    let result = large_items_body(context, arguments, &frame);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn large_items_body(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::MAP_PLACE_LARGE_LEVEL_ITEMS;
    let items = Address::<LevelItem>::new(arguments[0]);
    let level = arguments[1] as i16;
    let map_index = frame.local::<i16>(-8);
    map_index.write(
        context,
        if i16::from(ram::g_player1_map_level(revision).read(context)) == level {
            0
        } else {
            1
        },
    );
    let slot = frame.local::<i16>(-4);
    let attempts = frame.local::<i16>(-6);
    let current_y = frame.local::<i16>(-2);
    let x = frame.local::<i16>(-12);
    let y = frame.local::<i16>(-10);
    slot.write(context, 28);
    while slot.read(context) < 32 {
        attempts.write(context, 0);
        while attempts.read(context) < 10 {
            call_at(
                context,
                ids::MAP_FIND_ITEM_PLACEMENT_POSITION,
                &[x.address(), y.address(), level as u32, 1],
                CallSite::relative(caller, revision, 0x4e),
            )?;
            let mut blocked = false;
            for index in 28..i32::from(slot.read(context)) {
                let item = items.offset(index);
                let current_x = item.x().read(context);
                current_y.write(context, item.y().read(context));
                if (i32::from(x.read(context)) - i32::from(current_x)).wrapping_abs() < 340
                    && (i32::from(y.read(context)) - i32::from(current_y.read(context)))
                        .wrapping_abs()
                        < 150
                {
                    blocked = true;
                    break;
                }
            }
            if !blocked {
                for index in 0..28 {
                    let item = items.offset(index);
                    if item.kind().read(context) != -1 {
                        let current_x = item.x().read(context);
                        current_y.write(context, item.y().read(context));
                        if (i32::from(x.read(context)) - i32::from(current_x)).wrapping_abs() < 16
                            && (i32::from(y.read(context)) - i32::from(current_y.read(context)))
                                .wrapping_abs()
                                < 10
                        {
                            blocked = true;
                            break;
                        }
                    }
                }
            }
            if !blocked {
                for index in 0..2 {
                    let anchor = ram::g_level_anchor_positions(revision).at(index);
                    let map_x = anchor.x().read(context);
                    let map_y = anchor.y().read(context);
                    let current_x = (((i32::from(map_x) + 2) * 128) as i16).wrapping_sub(16);
                    current_y.write(context, ((i32::from(map_y) + 2) * 64) as i16);
                    current_y.write(context, current_y.read(context).wrapping_sub(5));
                    if (i32::from(x.read(context)) - i32::from(current_x)).wrapping_abs() < 340
                        && (i32::from(y.read(context)) - i32::from(current_y.read(context)))
                            .wrapping_abs()
                            < 150
                    {
                        blocked = true;
                        break;
                    }
                }
            }
            if !blocked {
                let args = [x.read(context) as u32, y.read(context) as u32, level as u32];
                if call_at(
                    context,
                    ids::EARTHLING_IS_SPECIAL_RECORD_NEARBY,
                    &args,
                    CallSite::relative(caller, revision, 0x1d2),
                )? != 0
                {
                    blocked = true;
                }
            }
            if !blocked {
                break;
            }
            attempts.write(context, attempts.read(context).wrapping_add(1));
        }
        let item = items.offset(i32::from(slot.read(context)));
        if attempts.read(context) == 10 {
            item.kind().write(context, -1);
        } else {
            let args = [
                map_index.read(context) as u32,
                x.read(context) as u32,
                y.read(context) as u32,
            ];
            // The TU's original int view consumes the whole narrow callee D0.
            // Its argument preparation leaves the promoted map index in D0.
            context.set_data_register(0, map_index.read(context) as u32);
            let terrain = call_at(
                context,
                ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
                &args,
                CallSite::relative(caller, revision, 0x228),
            )?;
            if terrain == 0x20 {
                let random = call_at(
                    context,
                    ids::GET_RANDOM_NUMBER,
                    &[],
                    CallSite::relative(caller, revision, 0x238),
                )?;
                item.kind()
                    .write(context, if random & 1 != 0 { 82 } else { 83 });
            } else {
                item.kind().write(context, 81);
            }
            item.level().write(context, level as i8);
            item.x().write(context, x.read(context));
            item.y().write(context, y.read(context));
        }
        slot.write(context, slot.read(context).wrapping_add(1));
    }
    Ok(None)
}

/// G/src/presents/level_item_setup.c:325 (`LoadLevelItemBank`).
/// Atomic: LINK -4; MOVEM D2-D6/A2-A5. Both coordinate locals retain the
/// original stack addresses. Call-PC offsets derive from object start 0x918.
pub fn load_level_item_bank(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x3c))?;
    let result = load_bank_body(context, arguments, &frame);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn load_bank_body(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::LOAD_LEVEL_ITEM_BANK;
    let level = arguments[0] as i16;
    if level >= 26 {
        return Ok(None);
    }
    for bank in 0..2 {
        let users = ram::g_item_bank_users(revision).at(bank);
        if users.read(context) != 0
            && i16::from(ram::g_item_bank_levels(revision).at(bank).read(context)) == level
        {
            users.write(context, users.read(context).wrapping_add(1));
            return Ok(None);
        }
    }
    let bank = if ram::g_item_bank_users(revision).at(0).read(context) != 0 {
        1
    } else {
        0
    };
    // ADD.W for the level seed and ASL.W #2 for collected bits, not full scale.
    let seed_address = ram::g_level_seeds(revision)
        .at(0)
        .byte_offset(i32::from(level.wrapping_mul(2)));
    let seed = seed_address.read(context);
    let saved_seed = call_at(
        context,
        ids::STACK_BASED_RANDOM,
        &[u32::from(seed)],
        CallSite::relative(caller, revision, 0x76),
    )?;
    let mut count = if ram::g_player_lives(revision).at(0).read(context) < 0
        || ram::g_player_lives(revision).at(1).read(context) < 0
    {
        12_i16
    } else {
        16
    };
    if level > 2 {
        count = count.wrapping_add(level.wrapping_sub(2));
    }
    if count > 28 {
        count = 28;
    }
    let x = frame.local::<i16>(-4);
    let y = frame.local::<i16>(-2);
    for slot in 0..i32::from(count) {
        if level == 0 {
            break;
        }
        if level == 1 {
            if slot == 12 {
                break;
            }
            let position = rom::level1_present_locations(revision).at(slot);
            x.write(context, position.x().read(context));
            y.write(context, position.y().read(context));
        } else {
            call_at(
                context,
                ids::MAP_FIND_ITEM_PLACEMENT_POSITION,
                &[x.address(), y.address(), level as u32, 0],
                CallSite::relative(caller, revision, 0xf4),
            )?;
        }
        let present = level == 1
            || (call_at(
                context,
                ids::GET_RANDOM_NUMBER,
                &[],
                CallSite::relative(caller, revision, 0x104),
            )? as u16)
                % 100
                < 50;
        let kind = if present {
            let mut kind = call_at(
                context,
                ids::SELECT_RANDOM_PRESENT,
                &[],
                CallSite::relative(caller, revision, 0x126),
            )? as i8;
            if level == 1 && slot < 4 {
                while matches!(kind, 0 | 2 | 5 | 26 | 18 | 16) {
                    kind = call_at(
                        context,
                        ids::SELECT_RANDOM_PRESENT,
                        &[],
                        CallSite::relative(caller, revision, 0x13e),
                    )? as i8;
                }
            }
            kind
        } else if (call_at(
            context,
            ids::GET_RANDOM_NUMBER,
            &[],
            CallSite::relative(caller, revision, 0x16a),
        )? as u16)
            % 100
            < 75
        {
            let food = (call_at(
                context,
                ids::GET_RANDOM_NUMBER,
                &[],
                CallSite::relative(caller, revision, 0x18e),
            )? as u16)
                % 16;
            (food as i8).wrapping_add(64)
        } else {
            80
        };
        let collected = ram::g_collected_level_item_bits(revision)
            .at(0)
            .byte_offset(i32::from(level.wrapping_shl(2)));
        let item = ram::g_object_table(revision).at(bank).at(slot);
        if collected.read(context) & tje_mem::arithmetic::lsr_u32(0x80000000, slot as u32) == 0 {
            item.kind().write(context, kind);
            item.level().write(context, level as i8);
            item.x().write(context, x.read(context));
            item.y().write(context, y.read(context));
        } else {
            item.kind().write(context, -1);
        }
    }
    let items = ram::g_object_table(revision).at(bank);
    if level == 0 {
        call_at(
            context,
            ids::INITIALIZE_LEVEL_ZERO_FIXED_ITEMS,
            &[items.at(28).address()],
            CallSite::relative(caller, revision, 0x23c),
        )?;
    } else if level != 1 {
        call_at(
            context,
            ids::MAP_PLACE_LARGE_LEVEL_ITEMS,
            &[items.address(), level as u32],
            CallSite::relative(caller, revision, 0x25a),
        )?;
    }
    let users = ram::g_item_bank_users(revision).at(bank);
    users.write(context, users.read(context).wrapping_add(1));
    ram::g_item_bank_levels(revision)
        .at(bank)
        .write(context, level as i8);
    call_at(
        context,
        ids::STACK_BASED_RANDOM,
        &[saved_seed],
        CallSite::relative(caller, revision, 0x27a),
    )?;
    Ok(None)
}

/// G/src/presents/telephone_open.c:9 (`TelephonePlaceOutsidePlayerViews`).
/// Atomic: LINK -4; MOVEM D2-D3/A2-A4, local X=-4/Y=-2. The four
/// placement arguments stay allocated through the first abs call, as in the
/// matching object's JSR at 0x9c and LEA 20(SP),SP at 0xb2.
pub fn telephone_place_outside_player_views(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x0c, 0x1c))?;
    let result = telephone_body(context, arguments, &frame);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn telephone_body(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::TELEPHONE_PLACE_OUTSIDE_PLAYER_VIEWS;
    let index = i32::from(arguments[0] as i16);
    let player = super::word_indexed_player(revision, index);
    // The promoted XOR expression takes the distinct ASL.L path.
    let other = if ram::g_player_lives(revision).at(index ^ 1).read(context) >= 0 {
        Some(ram::g_player_info(revision).at(index ^ 1))
    } else {
        None
    };
    let states = ram::g_telephone_states(revision);
    if states.at(index).level().read(context) == player.level().read(context)
        || states.at(index ^ 1).level().read(context) == player.level().read(context)
    {
        return Ok(None);
    }
    let telephone = states.at(index);
    let x = frame.local::<i16>(-4);
    let y = frame.local::<i16>(-2);
    loop {
        let values = [
            x.address(),
            y.address(),
            player.level().read(context) as u32,
            0,
        ];
        let outgoing = OutgoingArguments::push(context, &values)?;
        let dispatcher = context.dispatcher();
        outgoing.call(
            context,
            dispatcher,
            ids::MAP_FIND_ITEM_PLACEMENT_POSITION,
            CallSite::relative(caller, revision, 0xa2),
        )?;
        let delta = i32::from(player.x().read(context)) - i32::from(x.read(context));
        let distance_x = call_at(
            context,
            ids::ABS,
            &[delta as u32],
            CallSite::relative(caller, revision, 0xb2),
        )? as i32;
        outgoing.release(context)?;
        if distance_x < 480 {
            let delta = i32::from(player.y().read(context)) - i32::from(y.read(context));
            if (call_at(
                context,
                ids::ABS,
                &[delta as u32],
                CallSite::relative(caller, revision, 0xd0),
            )? as i32)
                < 336
            {
                continue;
            }
        }
        if let Some(other) = other {
            let delta = i32::from(other.x().read(context)) - i32::from(x.read(context));
            if (call_at(
                context,
                ids::ABS,
                &[delta as u32],
                CallSite::relative(caller, revision, 0xee),
            )? as i32)
                < 320
            {
                let delta = i32::from(other.y().read(context)) - i32::from(y.read(context));
                if (call_at(
                    context,
                    ids::ABS,
                    &[delta as u32],
                    CallSite::relative(caller, revision, 0x10a),
                )? as i32)
                    < 224
                {
                    continue;
                }
            }
        }
        break;
    }
    telephone
        .level()
        .write(context, player.level().read(context));
    telephone.x().write(context, x.read(context));
    telephone.y().write(context, y.read(context));
    ram::g_telephone_activated(revision)
        .at(index)
        .write(context, 0);
    Ok(None)
}
