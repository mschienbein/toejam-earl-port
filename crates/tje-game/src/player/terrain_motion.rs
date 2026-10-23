use super::frames::{call_from, with_frame};
use crate::call_frame::{FrameLayout, LocalFrame};
use crate::{Context, Result, ids};
use tje_mem::{Address, RenderFrame, Revision, ram};

/// G/src/player/entity_motion_terrain.c:71 (`PlayerProcessTerrainMotion`).
/// Matching c_entity_motion_terrain.s:86-87: LINK -16, MOVEM D2-D4/A2-A3.
/// The destination is A6-16: x/y at -16/-14, unused bytes -12/-11,
/// tile_x/y at -10/-8, fractions at -6/-5; A6-4..-1 remain untouched.
/// All return-PC offsets below were checked in both revisions' matching ELF.
pub(super) fn player_process_terrain_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    with_frame(
        context,
        FrameLayout::link(16, 0x1c, 0x0c),
        |context, stack| process(context, arguments, stack),
    )
}

fn process(
    context: &mut dyn Context,
    arguments: &[u32],
    stack: &LocalFrame,
) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let rendered = Address::<RenderFrame>::new(arguments[1]);
    let revision = context.revision();
    let caller = ids::PLAYER_PROCESS_TERRAIN_MOTION;
    if ram::g_player_lives(revision).at(number).read(context) < 0
        || ram::g_player_inventory_open(revision)
            .at(number)
            .read(context)
            != 0
    {
        return Ok(None);
    }
    let x = stack.local::<i16>(-16);
    let y = stack.local::<i16>(-14);
    let tile_x = stack.local::<i16>(-10);
    let tile_y = stack.local::<i16>(-8);
    let fraction_x = stack.local::<u8>(-6);
    let fraction_y = stack.local::<u8>(-5);
    x.write(context, -1);
    y.write(context, -1);
    let entity = super::indexing::player_by_word_index(revision, number);
    let paused = if revision == Revision::Rev02 {
        0x3a410
    } else {
        0x3a400
    };
    if entity.draw_flags().read(context) & 8 != 0
        || entity.animation_group_mask().read(context) & paused != 0
        || matches!(entity.sprite_state().read(context), 5 | 6)
    {
        return Ok(None);
    }
    if entity.animation_motion_axes().read(context) & 1 != 0 {
        let delta = (i32::from(entity.velocity_x().read(context))
            * i32::from(rendered.motion_x().read(context))) as i16;
        let fixed = (i32::from(entity.x().read(context)) << 8)
            .wrapping_add(i32::from(entity.fraction_x().read(context)))
            .wrapping_add(i32::from(delta));
        x.write(context, (fixed >> 8) as i16);
        fraction_x.write(context, fixed as u8);
        tile_x.write(
            context,
            ((i32::from(x.read(context)) >> 3) - i32::from(entity.x().read(context) >> 3) + 2)
                as i16,
        );
    } else {
        x.write(context, entity.x().read(context));
        fraction_x.write(context, entity.fraction_x().read(context));
        tile_x.write(context, 2);
    }
    if entity.animation_motion_axes().read(context) & 2 != 0 {
        let delta = (i32::from(entity.velocity_y().read(context))
            * i32::from(rendered.motion_y().read(context))) as i16;
        let fixed = (i32::from(entity.y().read(context)) << 8)
            .wrapping_add(i32::from(entity.fraction_y().read(context)))
            .wrapping_add(i32::from(delta));
        fraction_y.write(context, fixed as u8);
        y.write(context, (fixed >> 8) as i16);
        tile_y.write(
            context,
            ((i32::from(y.read(context)) >> 3) - i32::from(entity.y().read(context) >> 3) + 2)
                as i16,
        );
    } else {
        y.write(context, entity.y().read(context));
        fraction_y.write(context, entity.fraction_y().read(context));
        tile_y.write(context, 2);
    }
    entity
        .draw_flags()
        .write(context, entity.draw_flags().read(context) & !0x30);
    // This partner expression uses EXT.L/EOR.L/ASL.L, unlike the current
    // player's ASL.W/ADDA.W lookup (matching compiler lines 221-226).
    let other = ram::g_player_info(revision).at(number ^ 1);
    let partner_crossing = if entity.animation_group_mask().read(context) & 0x4400c != 0 {
        let args = [
            other.address(),
            entity.x().read(context) as u32,
            entity.y().read(context) as u32,
            entity.level().read(context) as u32,
        ];
        call_from(context, caller, ids::ENTITY_IS_NEAR_POSITION, &args, 0x19c)? == 0
            && ram::g_player_elevator_flags(revision)
                .at(number)
                .read(context)
                == 0
            && ram::g_player_elevator_flags(revision)
                .at(number ^ 1)
                .read(context)
                == 0
    } else {
        false
    };
    if partner_crossing {
        let args = [
            other.address(),
            x.read(context) as u32,
            entity.y().read(context) as u32,
            entity.level().read(context) as u32,
        ];
        if call_from(context, caller, ids::ENTITY_IS_NEAR_POSITION, &args, 0x1fa)? != 0 {
            x.write(context, entity.x().read(context));
            fraction_x.write(context, entity.fraction_x().read(context));
            tile_x.write(context, 2);
            entity
                .draw_flags()
                .write(context, entity.draw_flags().read(context) | 0x10);
        }
        let args = [
            other.address(),
            x.read(context) as u32,
            y.read(context) as u32,
            entity.level().read(context) as u32,
        ];
        if call_from(context, caller, ids::ENTITY_IS_NEAR_POSITION, &args, 0x244)? != 0 {
            y.write(context, entity.y().read(context));
            fraction_y.write(context, entity.fraction_y().read(context));
            tile_y.write(context, 2);
            entity
                .draw_flags()
                .write(context, entity.draw_flags().read(context) | 0x20);
        }
    }
    let args = [
        entity.address(),
        entity.x().read(context) as u32,
        entity.y().read(context) as u32,
        6,
    ];
    if call_from(
        context,
        caller,
        ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
        &args,
        0x27e,
    )? == 0
    {
        let args = [
            entity.address(),
            x.read(context) as u32,
            entity.y().read(context) as u32,
            6,
        ];
        if call_from(
            context,
            caller,
            ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
            &args,
            0x2a4,
        )? != 0
        {
            x.write(context, entity.x().read(context));
            fraction_x.write(context, entity.fraction_x().read(context));
            tile_x.write(context, 2);
            entity
                .draw_flags()
                .write(context, entity.draw_flags().read(context) | 0x10);
        }
        let args = [
            entity.address(),
            x.read(context) as u32,
            y.read(context) as u32,
            6,
        ];
        if call_from(
            context,
            caller,
            ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
            &args,
            0x2de,
        )? != 0
        {
            y.write(context, entity.y().read(context));
            fraction_y.write(context, entity.fraction_y().read(context));
            tile_y.write(context, 2);
            entity
                .draw_flags()
                .write(context, entity.draw_flags().read(context) | 0x20);
        }
    }
    let args = [
        number as u32,
        x.read(context) as u32,
        entity.y().read(context) as u32,
        1,
    ];
    if call_from(
        context,
        caller,
        ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
        &args,
        0x31e,
    )? != 0
    {
        x.write(context, entity.x().read(context));
        fraction_x.write(context, entity.fraction_x().read(context));
        tile_x.write(context, 2);
        entity
            .draw_flags()
            .write(context, entity.draw_flags().read(context) | 0x10);
    }
    let args = [
        number as u32,
        x.read(context) as u32,
        y.read(context) as u32,
        0,
    ];
    if call_from(
        context,
        caller,
        ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
        &args,
        0x35a,
    )? != 0
    {
        y.write(context, entity.y().read(context));
        fraction_y.write(context, entity.fraction_y().read(context));
        tile_y.write(context, 2);
        entity
            .draw_flags()
            .write(context, entity.draw_flags().read(context) | 0x20);
    }
    if entity.sprite_state().read(context) == 79 {
        if x.read(context) <= 28 || x.read(context) >= 2652 {
            x.write(context, entity.x().read(context));
            entity
                .draw_flags()
                .write(context, entity.draw_flags().read(context) | 0x10);
        }
        if y.read(context) <= 48 || y.read(context) >= 1736 {
            y.write(context, entity.y().read(context));
            entity
                .draw_flags()
                .write(context, entity.draw_flags().read(context) | 0x20);
        }
        entity.x().write(context, x.read(context));
        entity.fraction_x().write(context, fraction_x.read(context));
        entity.y().write(context, y.read(context));
        entity.fraction_y().write(context, fraction_y.read(context));
    } else if entity.sprite_state().read(context) != 78 {
        call_from(
            context,
            caller,
            ids::PLAYER_APPLY_TERRAIN_DESTINATION,
            &[number as u32, entity.address(), x.address()],
            0x3e8,
        )?;
    }
    Ok(None)
}
