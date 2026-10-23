use crate::{Context, Result, call, ids};
use tje_mem::{arithmetic, ram};

/// G/src/camera/split_views.c:12 (`AnimatePlayersIntoSplitViews`).
/// Real VBlank waits retain original interrupt work; 26.6 intermediates and the
/// original zero-distance divide trap are not replaced with a special case.
pub fn animate_players_into_split_views(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let first = ram::g_player_info(revision).at(0);
    let second = ram::g_player_info(revision).at(1);
    let saved = [
        first.x().read(context),
        first.y().read(context),
        second.x().read(context),
        second.y().read(context),
    ];
    let mut positions = [
        i32::from(first.x().read(context)).wrapping_shl(6),
        i32::from(first.y().read(context)).wrapping_shl(6),
        i32::from(second.x().read(context)).wrapping_shl(6),
        i32::from(second.y().read(context)).wrapping_shl(6),
    ];
    let targets = [
        (i32::from(ram::g_camera1_pending_x(revision).read(context)) + 160).wrapping_mul(64),
        (i32::from(ram::g_camera1_pending_y(revision).read(context))
            + 56
            + i32::from(first.height_pixels().read(context) / 2)
            + i32::from(first.height().read(context) / 2))
        .wrapping_mul(64),
        (i32::from(ram::g_camera1_pending_x(revision).read(context)) + 160).wrapping_mul(64),
        (i32::from(ram::g_camera1_pending_y(revision).read(context))
            + 168
            + i32::from(second.height_pixels().read(context) / 2)
            + i32::from(second.height().read(context) / 2))
        .wrapping_mul(64),
    ];
    let mut deltas =
        std::array::from_fn::<_, 4, _>(|index| targets[index].wrapping_sub(positions[index]));
    let mut distance = call(context, ids::ABS, &[deltas[0] as u32])? as i32;
    for delta in &deltas[1..] {
        let magnitude = call(context, ids::ABS, &[*delta as u32])? as i32;
        if distance < magnitude {
            distance = call(context, ids::ABS, &[*delta as u32])? as i32;
        }
    }
    let divisor = (distance >> 7).wrapping_add(192);
    distance = arithmetic::div_i32(distance, divisor)?;
    for delta in &mut deltas {
        *delta = arithmetic::div_i32(*delta, distance)?;
    }
    loop {
        let mut finished = true;
        let head = ram::g_entity_draw_list_head(revision)
            .read(context)
            .address();
        call(context, ids::POSITION_ENTITY_SPRITES, &[head])?;
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[1, 0])?;
        for index in 0..4 {
            let remaining = targets[index].wrapping_sub(positions[index]);
            if deltas[index] != 0 && remaining != 0 && (remaining ^ deltas[index]) >= 0 {
                finished = false;
                positions[index] = positions[index].wrapping_add(deltas[index]);
                let entity = if index < 2 { first } else { second };
                let coordinate = if index & 1 == 0 {
                    entity.x()
                } else {
                    entity.y()
                };
                coordinate.write(context, (positions[index] >> 6) as i16);
            }
        }
        if finished {
            break;
        }
    }
    first.x().write(context, saved[0]);
    first.y().write(context, saved[1]);
    second.x().write(context, saved[2]);
    second.y().write(context, saved[3]);
    Ok(None)
}

/// G/src/camera/split_screen_wipe.c:12 (`AnimateSplitScreenWipe`).
/// Both two-pass loops retain their final lower-row command, including pass two.
/// The allocation loop preserves entry D4: c_split_screen_wipe.s:4-46 does not
/// initialize it before AllocateEntityViewportSprites's residual-tile bug path.
pub fn animate_split_screen_wipe(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for player in 0..2 {
        let entity = ram::g_player_info(revision).at(player);
        while entity.draw_flags().read(context) & 2 == 0 {
            entity.sprite_ready().write(context, 0);
            call(
                context,
                ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                &[entity.address(), 0],
            )?;
            if entity.draw_flags().read(context) & 2 == 0 {
                call(context, ids::ENTITY_RELEASE_ONE_BY_PRIORITY, &[])?;
            }
        }
    }
    let first = ram::g_player_info(revision).at(0);
    let second = ram::g_player_info(revision).at(1);
    while first.sprite_ready().read(context) == 0 || second.sprite_ready().read(context) == 0 {
        call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
    }
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    call(context, ids::LINK_PLAYER_SPRITES_FOR_SPLIT_WIPE, &[])?;
    let cursor = ram::g_camera1_tile_row_cursor(revision);
    let mut upper = cursor.read(context) & 31;
    let mut lower = cursor.read(context).wrapping_add(29) & 31;
    for step in 0..15 {
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        context.write_vdp_command((upper as u32).wrapping_shl(23).wrapping_add(0x60000002));
        for _ in 0..2 {
            for _ in 0..64 {
                context.write_vdp_data(0x25b4);
            }
            context.write_vdp_command((lower as u32).wrapping_shl(23).wrapping_add(0x60000002));
        }
        if step == 14 {
            break;
        }
        upper = upper.wrapping_add(1) & 31;
        lower = lower.wrapping_sub(1) & 31;
        context.write_vdp_command((upper as u32).wrapping_shl(23).wrapping_add(0x60000002));
        for _ in 0..2 {
            for _ in 0..64 {
                context.write_vdp_data(0x05ab);
            }
            context.write_vdp_command((lower as u32).wrapping_shl(23).wrapping_add(0x60000002));
        }
    }
    call(context, ids::SELECT_SPRITE_CHAIN_START, &[1])?;
    for row in 0..28 {
        context.write_vdp_command((row as u32).wrapping_shl(23).wrapping_add(0x50000003));
        for _ in 0..40 {
            context.write_vdp_data(0xa5b4);
        }
    }
    context.write_vdp_control(0x921c);
    Ok(None)
}
