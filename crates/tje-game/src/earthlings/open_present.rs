use crate::call_frame::{FrameLayout, LocalFrame};
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// G/src/earthlings/spawning_and_banks.c:476 (`OpenEarthlingPresent`).
/// Atomic: direct RNG, map queries and object/record RAM. Preserve both exhausted
/// scan bugs: read the one-past-bank record, and write it if the second scan fails.
/// The compiler's initial word read followed by EXT.W discards its old high byte.
pub fn open_earthling_present(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0x7c, 0x1c))?;
    let result = open_present(context, arguments, &frame);
    super::framed::finish(context, frame, result)
}

fn open_present(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(arguments[0]);
    frame.local::<i16>(-4).write(context, 0);
    frame
        .local::<i8>(-5)
        .write(context, player.entity_index().read(context));
    // Both matching listings c_spawning_and_banks.s:1431-1434 write only A6-5,
    // then read the mixed word A6-6 (entry SP-10). EXT.W kills its unwritten
    // high byte; declare that actual read rather than inventing a high-byte store.
    let player_number = i16::from(context.read_uninitialized_local(
        frame.entry().uninitialized_local(
            ids::OPEN_EARTHLING_PRESENT,
            -10,
            crate::stack_read::LocalWidth::Word,
            "G/build/{rev00,rev02}/asm/c_spawning_and_banks.s:1431-1434; LINK -8; MOVE.B 94(A2),-5(A6); MOVE.W -6(A6),D0; EXT.W D0; own entry SP-10 word",
        ),
    )? as i8);
    frame.local::<i16>(-6).write(context, player_number);
    let mut terrain = arguments[1] as i16;
    let mut x = 0_i16;
    let mut y = 0_i16;
    let mut selection = 0_i16;
    for _ in 0..4 {
        x = player.x().read(context);
        if (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) & 1 != 0 {
            x = x.wrapping_add(40)
        } else {
            x = x.wrapping_sub(40)
        }
        y = player.y().read(context);
        frame.local::<i16>(-2).write(context, y);
        if (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) & 1 != 0 {
            y = y.wrapping_add(25)
        } else {
            y = y.wrapping_sub(25)
        }
        frame.local::<i16>(-2).write(context, y);
        let terrain_player = if player.draw_flags().read(context) & 2 != 0 {
            0
        } else {
            1
        };
        selection = call(
            context,
            ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
            &[terrain_player, x as u32, y as u32],
        )? as i16;
        if (terrain == 0x80 && selection == 0x80) || (terrain != 0x80 && selection != 0x80) {
            break;
        }
    }
    terrain = selection;
    let bank = if ram::g_earthling_bank_users(revision).at(0).read(context) == 0
        || ram::g_earthling_bank_levels(revision).at(0).read(context)
            != player.level().read(context)
    {
        1
    } else {
        0
    };
    frame.local::<i16>(-4).write(context, bank as i16);
    let records = ram::g_earthling_banks(revision).at(bank);
    let mut earthling = records.at(0);
    for _ in 0..20 {
        if earthling.kind().read(context) == -1 {
            break;
        }
        earthling = earthling.offset(1);
    }
    if earthling.kind().read(context) != -1 {
        earthling = records.at(0);
        for _ in 0..20 {
            if earthling.kind().read(context) != 0
                && earthling.kind().read(context) != 9
                && earthling.object().read(context) == 0
            {
                break;
            }
            earthling = earthling.offset(1);
        }
    }
    if player.level().read(context) == 0 {
        earthling.kind().write(context, 5)
    } else if terrain == 0x80 {
        earthling.kind().write(context, 14)
    } else if terrain == 0x90 || terrain == 0x50 {
        earthling.kind().write(context, 22)
    } else {
        loop {
            let kind = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 22 + 1) as i8;
            earthling.kind().write(context, kind);
            if [17, 20, 11, 10, 9].contains(&kind) {
                let mut found = false;
                let mut selected = false;
                for slot in 0..20 {
                    let other = records.at(slot);
                    if earthling != other
                        && other.kind().read(context) == earthling.kind().read(context)
                    {
                        if other.object().read(context) == 0 {
                            earthling = other;
                            selected = true;
                        }
                        found = true;
                        break;
                    }
                }
                if !found {
                    if earthling.kind().read(context) != 9 {
                        selected = true
                    } else {
                        let level = player.level().read(context) as u32;
                        selected = call(
                            context,
                            ids::EARTHLING_CONFIGURE_MAILBOX_LEVEL_BANK,
                            &[bank as u32, level],
                        )? != 0;
                    }
                }
                if selected {
                    break;
                }
            } else if earthling.kind().read(context) != 14
                && (earthling.kind().read(context) != 8 || terrain == 0x20)
                && (earthling.kind().read(context) != 13 || terrain == 0x10)
            {
                break;
            }
        }
    }
    earthling
        .level()
        .write(context, player.level().read(context));
    earthling.x().write(context, x);
    earthling.y().write(context, y);
    earthling.saved_y().write(context, y);
    let mut height = 5;
    if earthling.kind().read(context) == 18 || earthling.kind().read(context) == 19 {
        earthling.saved_x_or_status().write(context, 0);
        earthling.saved_y().write(context, 0);
    } else if earthling.kind().read(context) == 7 {
        earthling.saved_x_or_status().write(context, 1);
        height = 30;
    } else {
        earthling.saved_x_or_status().write(context, x)
    }
    let definition =
        rom::earthling_definitions(revision).at(i32::from(earthling.kind().read(context)));
    earthling
        .health_visibility()
        .at(0)
        .write(context, definition.health().read(context));
    earthling.health_visibility().at(1).write(context, 0);
    let effect_x = earthling.x().read(context) as u32;
    let effect_y = i32::from(earthling.y().read(context)) + 1;
    let level = earthling.level().read(context) as u32;
    call(
        context,
        ids::PRESENT_START_TRANSIENT_EFFECT,
        &[
            player_number as u32,
            effect_x,
            effect_y as u32,
            height,
            level,
        ],
    )?;
    let object = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
    if object != 0 {
        call_void(context, ids::FREE_ENTITY, &[object as u32])?;
    } else {
        let object = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[1])? as i16;
        if object != 0 {
            call_void(context, ids::FREE_ENTITY, &[object as u32])?;
        }
    }
    Ok(None)
}
